/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! Embedded Postgres lifecycle. Per-query traffic is [`super::postgres`].

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
mod cluster {
    use std::path::{Path, PathBuf};

    use moqui_lib::error::Error;
    use postgresql_embedded::blocking::PostgreSQL;
    use postgresql_embedded::{Settings, BOOTSTRAP_SUPERUSER};

    use super::super::postgres::PostgresDriver;

    const DATABASE: &str = "moqui";
    const PORT: u16 = 5432;
    /// `.cargo/config.toml` forces `POSTGRESQL_VERSION` to this exact theseus build.
    const SHIPPED_VERSION: &str = "18.6.0";
    const SHIPPED_MAJOR: u32 = 18;
    const CLI_MAX_CONNECTIONS: u32 = 20;

    /// One theseus postmaster. The install and data directory stay under the caller's path.
    pub struct EmbeddedCluster {
        /// `PostgreSQL` stops a started server when it drops. An adopted one must outlive us.
        server: std::mem::ManuallyDrop<PostgreSQL>,
        data_dir: PathBuf,
        stopped: bool,
        /// A postmaster another process started. This handle never stops it.
        adopted: bool,
    }

    struct PostmasterLock {
        pid: u32,
        port: u16,
    }

    impl EmbeddedCluster {
        /// Install, initdb, start, and create the `moqui` database.
        ///
        /// On Unix the socket directory is the data directory and `listen_addresses` is empty.
        /// On Windows `listen_addresses` is `127.0.0.1` and the postmaster binds an ephemeral port.
        /// The client uses `NoTls` either way.
        ///
        /// A live postmaster is adopted when `PG_VERSION` matches this theseus major, and refused
        /// with that pid when it does not. A dead pid is not live. A stopped data directory whose
        /// major differs is an error: this build ships theseus 18.6.0 and does not run `pg_upgrade`.
        /// A second start reloads `pgpass` and skips `CREATE DATABASE` when `moqui` already exists.
        ///
        /// # Errors
        ///
        /// Returns an error when setup, start, the version check, or `CREATE DATABASE` fails.
        pub fn start(dir: impl AsRef<Path>, max_connections: u32) -> Result<(Self, PostgresDriver), Error> {
            let dir = dir.as_ref();
            std::fs::create_dir_all(dir).map_err(|err| Error::new("create embedded dir").with_source(err))?;
            let data_dir = dir.join("data");
            #[cfg(unix)]
            check_socket_len(&data_dir)?;
            if let Some(lock) = postmaster_lock(&data_dir)? {
                if process_live(lock.pid) {
                    return adopt(dir, &data_dir, max_connections, lock);
                }
            }
            ensure_major_can_start(dir, &data_dir)?;
            launch(dir, &data_dir, max_connections, None)
        }

        /// `pg_dump --format=custom` of the whole `moqui` database, including a `search` schema.
        ///
        /// # Errors
        ///
        /// Returns an error when `pg_dump` is missing or exits non-zero.
        pub fn save_db(&self, dump: &Path) -> Result<(), Error> {
            let mut args = connect_args(self);
            args.push("--format=custom".to_string());
            args.push("--file".to_string());
            args.push(dump.display().to_string());
            run_tool(self, "pg_dump", &args)
        }

        /// `pg_restore --format=custom` of a dump from [`Self::save_db`].
        ///
        /// # Errors
        ///
        /// Returns an error when `pg_restore` is missing or exits non-zero.
        pub fn load_save(&self, dump: &Path) -> Result<(), Error> {
            let mut args = connect_args(self);
            args.push("--format=custom".to_string());
            args.push("--clean".to_string());
            args.push("--if-exists".to_string());
            args.push(dump.display().to_string());
            run_tool(self, "pg_restore", &args)
        }

        /// Stop the postmaster. A second call is a no-op, and so is a call on an adopted postmaster.
        ///
        /// # Errors
        ///
        /// Returns an error when `pg_ctl stop` fails.
        pub fn stop(&mut self) -> Result<(), Error> {
            if self.stopped || self.adopted {
                return Ok(());
            }
            self.server
                .stop()
                .map_err(|err| embedded("stop embedded postgres", &self.data_dir, err))?;
            self.stopped = true;
            Ok(())
        }

        /// Data directory. On Unix this is also the socket directory.
        #[must_use]
        pub fn data_dir(&self) -> &Path {
            &self.data_dir
        }
    }

    impl Drop for EmbeddedCluster {
        fn drop(&mut self) {
            let _ = self.stop();
            if !self.adopted {
                // SAFETY: `server` is dropped once, here, and never used again.
                unsafe { std::mem::ManuallyDrop::drop(&mut self.server) };
            }
        }
    }

    /// Start the cluster, dump it, and stop it. A live postmaster that was adopted keeps running.
    ///
    /// # Errors
    ///
    /// Returns an error when the cluster or `pg_dump` fails.
    pub fn save_db(dir: &Path, dump: &Path) -> Result<(), Error> {
        let (cluster, _driver) = EmbeddedCluster::start(dir, CLI_MAX_CONNECTIONS)?;
        cluster.save_db(dump)
    }

    /// Start the cluster, restore a custom-format dump, and stop it. A live postmaster that was
    /// adopted keeps running.
    ///
    /// # Errors
    ///
    /// Returns an error when the cluster or `pg_restore` fails.
    pub fn load_save(dir: &Path, dump: &Path) -> Result<(), Error> {
        let (cluster, _driver) = EmbeddedCluster::start(dir, CLI_MAX_CONNECTIONS)?;
        cluster.load_save(dump)
    }

    #[cfg(unix)]
    fn check_socket_len(data_dir: &Path) -> Result<(), Error> {
        let socket_file = data_dir.join(format!(".s.PGSQL.{PORT}"));
        if socket_file.as_os_str().len() >= 108 {
            return Err(Error::new(format!(
                "postgres socket path is {} bytes; unix sockets need fewer than 108",
                socket_file.as_os_str().len()
            )));
        }
        Ok(())
    }

    fn adopt(
        dir: &Path,
        data_dir: &Path,
        max_connections: u32,
        lock: PostmasterLock,
    ) -> Result<(EmbeddedCluster, PostgresDriver), Error> {
        let major = pg_major(data_dir)?.unwrap_or(0);
        if major != SHIPPED_MAJOR {
            return Err(Error::new(format!(
                "postmaster {} is live for {} and PG_VERSION is {major}; theseus {SHIPPED_VERSION} will not adopt it or start a second postmaster",
                lock.pid,
                data_dir.display()
            )));
        }
        launch(dir, data_dir, max_connections, Some(lock.port))
    }

    fn ensure_major_can_start(dir: &Path, data_dir: &Path) -> Result<(), Error> {
        let Some(major) = pg_major(data_dir)? else {
            return Ok(());
        };
        if major == SHIPPED_MAJOR {
            return Ok(());
        }
        if old_install_present(dir) {
            Err(Error::new(format!(
                "PG_VERSION is {major} and this build only ships theseus {SHIPPED_VERSION}; pg_upgrade will not run"
            )))
        } else {
            Err(Error::new(format!(
                "PG_VERSION is {major} and theseus {SHIPPED_VERSION} has no previous binaries beside the install"
            )))
        }
    }

    fn launch(
        dir: &Path,
        data_dir: &Path,
        max_connections: u32,
        adopt_port: Option<u16>,
    ) -> Result<(EmbeddedCluster, PostgresDriver), Error> {
        let settings = settings(dir, data_dir, max_connections, adopt_port);
        let mut server = PostgreSQL::new(settings);
        server
            .setup()
            .map_err(|err| embedded("setup embedded postgres", data_dir, err))?;
        if adopt_port.is_none() {
            server
                .start()
                .map_err(|err| embedded("start embedded postgres", data_dir, err))?;
        }
        let data_dir = server.settings().data_dir.clone();
        let exists = server
            .database_exists(DATABASE)
            .map_err(|err| embedded("check embedded database", &data_dir, err))?;
        if !exists {
            server
                .create_database(DATABASE)
                .map_err(|err| embedded("create embedded database", &data_dir, err))?;
        }
        let password = server.settings().password.clone();
        let driver = driver_for(&server, &password);
        Ok((
            EmbeddedCluster {
                server: std::mem::ManuallyDrop::new(server),
                data_dir,
                stopped: false,
                adopted: adopt_port.is_some(),
            },
            driver,
        ))
    }

    fn driver_for(server: &PostgreSQL, password: &str) -> PostgresDriver {
        let settings = server.settings();
        #[cfg(unix)]
        {
            let socket = settings.socket_dir.clone().unwrap_or_else(|| settings.data_dir.clone());
            PostgresDriver::unix_socket(BOOTSTRAP_SUPERUSER, DATABASE, socket).password(password)
        }
        #[cfg(not(unix))]
        {
            PostgresDriver::tcp_no_tls(BOOTSTRAP_SUPERUSER, DATABASE, settings.host.clone(), settings.port)
                .password(password)
        }
    }

    fn settings(dir: &Path, data_dir: &Path, max_connections: u32, adopt_port: Option<u16>) -> Settings {
        let mut settings = Settings::new();
        settings.installation_dir = dir.join("install");
        settings.password_file = dir.join("pgpass");
        settings.data_dir = data_dir.to_path_buf();
        settings.temporary = false;
        settings.timeout = None;
        // fsync=on overrides pg_ctl's `-F`.
        settings
            .configuration
            .insert("shared_buffers".to_string(), "16MB".to_string());
        settings.configuration.insert("fsync".to_string(), "on".to_string());
        settings
            .configuration
            .insert("logging_collector".to_string(), "off".to_string());
        settings
            .configuration
            .insert("max_connections".to_string(), max_connections.to_string());
        #[cfg(unix)]
        {
            settings.socket_dir = Some(data_dir.to_path_buf());
            settings.port = adopt_port.unwrap_or(PORT);
            // Empty listen_addresses disables TCP.
            settings
                .configuration
                .insert("listen_addresses".to_string(), String::new());
        }
        #[cfg(not(unix))]
        {
            settings.socket_dir = None;
            settings.host = "127.0.0.1".to_string();
            // Port 0 asks postgresql_embedded to bind an ephemeral port.
            settings.port = adopt_port.unwrap_or(0);
            settings
                .configuration
                .insert("listen_addresses".to_string(), "127.0.0.1".to_string());
        }
        reuse_password(&mut settings);
        settings
    }

    fn reuse_password(settings: &mut Settings) {
        let Ok(text) = std::fs::read_to_string(&settings.password_file) else {
            return;
        };
        let text = text.trim();
        if !text.is_empty() {
            settings.password = text.to_string();
        }
    }

    fn pg_major(data_dir: &Path) -> Result<Option<u32>, Error> {
        let path = data_dir.join("PG_VERSION");
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|err| Error::new(format!("read {}", path.display())).with_source(err))?;
        let major = text
            .trim()
            .parse::<u32>()
            .map_err(|_| Error::new(format!("PG_VERSION {:?} is not a major version", text.trim())))?;
        Ok(Some(major))
    }

    fn postmaster_lock(data_dir: &Path) -> Result<Option<PostmasterLock>, Error> {
        let path = data_dir.join("postmaster.pid");
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path).map_err(|err| Error::new("read postmaster.pid").with_source(err))?;
        let mut lines = text.lines();
        let pid_line = lines.next().unwrap_or("").trim();
        let pid: u32 = pid_line
            .parse()
            .map_err(|_| Error::new(format!("postmaster.pid pid {pid_line:?} is not a pid")))?;
        let _data_directory = lines.next();
        let _started = lines.next();
        let port_line = lines
            .next()
            .ok_or_else(|| Error::new("postmaster.pid has no port"))?
            .trim();
        let port: u16 = port_line
            .parse()
            .map_err(|_| Error::new(format!("postmaster.pid port {port_line:?} is not a port")))?;
        Ok(Some(PostmasterLock { pid, port }))
    }

    fn process_live(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        #[cfg(target_os = "linux")]
        {
            linux_process_live(pid)
        }
        #[cfg(all(unix, not(target_os = "linux")))]
        {
            std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .is_ok_and(|status| status.success())
        }
        #[cfg(windows)]
        {
            windows_process_live(pid)
        }
    }

    #[cfg(target_os = "linux")]
    fn linux_process_live(pid: u32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let Some(end) = stat.rfind(')') else {
            return false;
        };
        let state = stat[end + 1..].split_whitespace().next().unwrap_or("");
        !state.is_empty() && state != "Z"
    }

    /// `tasklist` is the process probe that does not add a Windows crate. This host is Linux, so
    /// the TCP start path is not executed here.
    #[cfg(windows)]
    fn windows_process_live(pid: u32) -> bool {
        let Ok(output) = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
        else {
            return false;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        text.contains(&pid.to_string()) && !text.to_ascii_lowercase().contains("no tasks")
    }

    fn old_install_present(cluster_dir: &Path) -> bool {
        // `PostgreSQL::new` appends the exact version, so this build's binaries live in
        // `install/18.6.0`. Anything inside that tree is this theseus install.
        let shipped_root = cluster_dir.join("install");
        let shipped = shipped_root.join(SHIPPED_VERSION);
        other_server(cluster_dir, &shipped, &shipped_root) || other_server(&shipped_root, &shipped, &shipped_root)
    }

    fn other_server(dir: &Path, shipped: &Path, shipped_root: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == *shipped || path == *shipped_root || path.starts_with(shipped) || !path.is_dir() {
                continue;
            }
            if has_server_bin(&path) {
                return true;
            }
        }
        false
    }

    fn has_server_bin(dir: &Path) -> bool {
        let bin = dir.join("bin");
        bin.join("postgres").is_file()
            || bin.join("pg_upgrade").is_file()
            || bin.join("postgres.exe").is_file()
            || bin.join("pg_upgrade.exe").is_file()
    }

    fn connect_args(cluster: &EmbeddedCluster) -> Vec<String> {
        let settings = cluster.server.settings();
        let mut args = vec![
            "--username".to_string(),
            BOOTSTRAP_SUPERUSER.to_string(),
            "--port".to_string(),
            settings.port.to_string(),
            "--dbname".to_string(),
            DATABASE.to_string(),
            "--no-password".to_string(),
        ];
        #[cfg(unix)]
        if let Some(socket) = &settings.socket_dir {
            args.push("--host".to_string());
            args.push(socket.display().to_string());
        }
        #[cfg(not(unix))]
        {
            args.push("--host".to_string());
            args.push(settings.host.clone());
        }
        args
    }

    fn tool_bin(bin_dir: &Path, name: &str) -> PathBuf {
        #[cfg(windows)]
        {
            bin_dir.join(format!("{name}.exe"))
        }
        #[cfg(not(windows))]
        {
            bin_dir.join(name)
        }
    }

    fn run_tool(cluster: &EmbeddedCluster, name: &str, args: &[String]) -> Result<(), Error> {
        let bin = tool_bin(&cluster.server.settings().binary_dir(), name);
        let output = std::process::Command::new(&bin)
            .args(args)
            .env("PGPASSWORD", &cluster.server.settings().password)
            .output()
            .map_err(|err| Error::new(format!("run {}", bin.display())).with_source(err))?;
        if output.status.success() {
            return Ok(());
        }
        let detail = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        Err(Error::new(format!("{name} exited {}: {detail}", output.status)))
    }

    fn embedded(context: &str, data_dir: &Path, err: postgresql_embedded::Error) -> Error {
        let log = log_tail(&data_dir.join("start.log"));
        let message = if log.is_empty() {
            context.to_string()
        } else {
            format!("{context}: {log}")
        };
        Error::new(message).with_source(err)
    }

    fn log_tail(path: &Path) -> String {
        let Ok(text) = std::fs::read_to_string(path) else {
            return String::new();
        };
        let mut start = text.len().saturating_sub(1_500);
        while start < text.len() && !text.is_char_boundary(start) {
            start += 1;
        }
        text[start..].to_string()
    }

    #[cfg(all(test, unix))]
    mod tests {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        use std::fmt::Write as _;
        use std::path::{Path, PathBuf};

        use moqui_core::driver::{DbConnection, DbDriver};
        use moqui_lib::object::{basic_convert, Value};

        use super::super::super::postgres::PostgresConn;
        use super::*;

        struct DirGuard(PathBuf);

        impl Drop for DirGuard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        #[test]
        fn embedded_cluster_queries_stream_cache_and_drops() {
            let dir = temp_cluster_dir();
            let _guard = DirGuard(dir.clone());
            // theseus 18.6.0 NEEDED libxml2.so.2. A host that only ships .so.16 cannot run initdb.
            provide_libxml2_soname(&dir);
            let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
            let data_dir = cluster.data_dir().to_path_buf();
            assert!(
                data_dir.join(".s.PGSQL.5432").exists(),
                "socket missing in {}",
                data_dir.display()
            );

            let mut conn = driver.connect_postgres().unwrap();
            assert_eq!(show(&mut conn, "fsync"), "on");
            assert_eq!(show(&mut conn, "logging_collector"), "off");
            assert_eq!(show(&mut conn, "max_connections"), "20");
            assert_eq!(show(&mut conn, "listen_addresses"), "");
            let buffers = show(&mut conn, "shared_buffers");
            assert!(buffers == "16MB" || buffers == "16384kB", "shared_buffers={buffers}");

            let selected = conn.query_once("SELECT 1", &[]).unwrap();
            assert_eq!(selected.len(), 1);
            assert_eq!(selected[0].columns[0].1, Value::I32(1));

            let mut boxed = driver.connect().unwrap();
            let boxed_rows = boxed.query_once("SELECT 1", &[]).unwrap();
            assert_eq!(boxed_rows[0].columns[0].1, Value::I32(1));
            drop(boxed);

            conn.execute("CREATE TABLE note (body text)", &[]).unwrap();
            let inserted = conn
                .execute("INSERT INTO note (body) VALUES (?)", &[Value::string("hello?")])
                .unwrap();
            assert_eq!(inserted, 1);
            let notes = conn
                .query(
                    "SELECT body FROM note WHERE body = ? AND '?' = '?'",
                    &[Value::string("hello?")],
                )
                .unwrap();
            assert_eq!(notes.len(), 1);
            assert_eq!(notes[0].get("body"), Some(&Value::string("hello?")));

            round_trip_scalars(&mut conn);
            savepoint_keeps_the_first_row(&mut conn);
            stream_requires_a_transaction(&mut conn);
            stream_reads_every_inserted_row(&mut conn);

            let mut other = driver.connect_postgres().unwrap();
            conn.begin().unwrap();
            conn.execute("INSERT INTO note (body) VALUES (?)", &[Value::string("hidden")])
                .unwrap();
            let unseen = other
                .query("SELECT body FROM note WHERE body = ?", &[Value::string("hidden")])
                .unwrap();
            assert!(unseen.is_empty(), "{unseen:?}");
            conn.commit().unwrap();
            let seen = other
                .query("SELECT body FROM note WHERE body = ?", &[Value::string("hidden")])
                .unwrap();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].get("body"), Some(&Value::string("hidden")));
            drop(other);

            let mut cached = driver.connect_postgres().unwrap();
            assert_eq!(cached.statement_cache_len(), 0);
            cached.query("SELECT 1", &[]).unwrap();
            cached.query("SELECT 1", &[]).unwrap();
            assert_eq!(cached.statement_cache_len(), 1);
            cached.query_once("SELECT 1", &[]).unwrap();
            cached.query_once("SELECT 2", &[]).unwrap();
            assert_eq!(cached.statement_cache_len(), 1);
            cached.canceller().cancel().unwrap();
            drop(cached);
            drop(conn);

            let pid = postmaster_pid(&data_dir);
            drop(cluster);
            assert!(
                !data_dir.join("postmaster.pid").exists(),
                "postmaster.pid still present"
            );
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "postmaster {pid} still alive"
            );
        }

        #[test]
        fn same_version_restart_keeps_the_row() {
            let dir = prepared_dir();
            let _guard = DirGuard(dir.clone());
            {
                let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
                let mut conn = driver.connect_postgres().unwrap();
                conn.execute("CREATE TABLE kept (n int)", &[]).unwrap();
                conn.execute("INSERT INTO kept (n) VALUES (?)", &[Value::I32(7)])
                    .unwrap();
                drop(conn);
                drop(cluster);
            }
            let (_cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
            let mut conn = driver.connect_postgres().unwrap();
            let rows = conn.query("SELECT n FROM kept", &[]).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get("n"), Some(&Value::I32(7)));
        }

        #[test]
        fn adopt_live_postmaster_keeps_the_same_pid() {
            let dir = prepared_dir();
            let _guard = DirGuard(dir.clone());
            let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
            let data_dir = cluster.data_dir().to_path_buf();
            drop(driver);
            let pid = postmaster_pid(&data_dir);
            let _kill = KillGuard(pid);
            std::mem::forget(cluster);
            let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
            assert_eq!(postmaster_pid(&data_dir), pid);
            let mut conn = driver.connect_postgres().unwrap();
            let rows = conn.query_once("SELECT 1", &[]).unwrap();
            assert_eq!(rows[0].columns[0].1, Value::I32(1));
            drop(conn);
            drop(cluster);
            assert_eq!(postmaster_pid(&data_dir), pid, "adopted postmaster was stopped");
            assert!(
                Path::new(&format!("/proc/{pid}")).exists(),
                "adopted postmaster {pid} must keep running"
            );
        }

        #[test]
        fn live_major_mismatch_refuses_with_the_pid() {
            let dir = prepared_dir();
            let _guard = DirGuard(dir.clone());
            let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
            let data_dir = cluster.data_dir().to_path_buf();
            drop(driver);
            let pid = postmaster_pid(&data_dir);
            let _kill = KillGuard(pid);
            std::mem::forget(cluster);
            std::fs::write(data_dir.join("PG_VERSION"), "17\n").unwrap();
            let err = start_error(&dir);
            assert!(err.message().contains(&pid.to_string()), "{err}");
            assert!(err.message().contains("17"), "{err}");
            assert!(err.message().contains(SHIPPED_VERSION), "{err}");
            assert!(
                Path::new(&format!("/proc/{pid}")).exists(),
                "refusing to adopt killed postmaster {pid}"
            );
        }

        #[test]
        fn dead_pid_starts_a_new_postmaster() {
            let dir = prepared_dir();
            let _guard = DirGuard(dir.clone());
            let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
            let data_dir = cluster.data_dir().to_path_buf();
            drop(driver);
            let pid = postmaster_pid(&data_dir);
            let _kill = KillGuard(pid);
            std::mem::forget(cluster);
            kill_pid(pid);
            let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
            let mut conn = driver.connect_postgres().unwrap();
            let rows = conn.query_once("SELECT 1", &[]).unwrap();
            assert_eq!(rows[0].columns[0].1, Value::I32(1));
            drop(conn);
            drop(cluster);
        }

        #[test]
        fn stopped_major_mismatch_names_the_missing_binaries() {
            let dir = prepared_dir();
            let _guard = DirGuard(dir.clone());
            let (cluster, _driver) = EmbeddedCluster::start(&dir, 20).unwrap();
            let data_dir = cluster.data_dir().to_path_buf();
            drop(cluster);
            std::fs::write(data_dir.join("PG_VERSION"), "17\n").unwrap();
            let err = start_error(&dir);
            assert!(err.message().contains("17"), "{err}");
            assert!(err.message().contains(SHIPPED_VERSION), "{err}");
            assert!(err.message().contains("binaries"), "{err}");
        }

        #[test]
        fn stopped_major_mismatch_with_a_sibling_install_does_not_upgrade() {
            let dir = prepared_dir();
            let _guard = DirGuard(dir.clone());
            let (cluster, _driver) = EmbeddedCluster::start(&dir, 20).unwrap();
            let data_dir = cluster.data_dir().to_path_buf();
            drop(cluster);
            std::fs::write(data_dir.join("PG_VERSION"), "17\n").unwrap();
            let upgrade = dir.join("postgresql-17").join("bin").join("pg_upgrade");
            std::fs::create_dir_all(upgrade.parent().unwrap()).unwrap();
            std::fs::write(&upgrade, b"").unwrap();
            let err = start_error(&dir);
            assert!(err.message().contains("17"), "{err}");
            assert!(err.message().contains(SHIPPED_VERSION), "{err}");
            assert!(err.message().contains("pg_upgrade"), "{err}");
            assert_eq!(std::fs::read(&upgrade).unwrap(), b"");
        }

        #[test]
        fn save_db_round_trips_the_search_schema() {
            let dir = prepared_dir();
            let _guard = DirGuard(dir.clone());
            let (cluster, driver) = EmbeddedCluster::start(&dir, 20).unwrap();
            let mut conn = driver.connect_postgres().unwrap();
            conn.execute("CREATE SCHEMA search", &[]).unwrap();
            conn.execute("CREATE TABLE search.kept (body text)", &[]).unwrap();
            conn.execute("INSERT INTO search.kept (body) VALUES (?)", &[Value::string("kept")])
                .unwrap();
            let dump = dir.join("moqui.dump");
            cluster.save_db(&dump).unwrap();
            let header = std::fs::read(&dump).unwrap();
            assert!(header.starts_with(b"PGDMP"), "dump is not custom format");
            let list = std::process::Command::new(dir.join("install").join(SHIPPED_VERSION).join("bin/pg_restore"))
                .arg("--list")
                .arg(&dump)
                .output()
                .unwrap();
            assert!(list.status.success(), "{}", String::from_utf8_lossy(&list.stderr));
            let listed = String::from_utf8_lossy(&list.stdout);
            assert!(listed.contains("search"), "{listed}");
            drop(conn);
            conn_drop_schema(&driver);
            cluster.load_save(&dump).unwrap();
            let mut conn = driver.connect_postgres().unwrap();
            let rows = conn.query("SELECT body FROM search.kept", &[]).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get("body"), Some(&Value::string("kept")));
        }

        fn start_error(dir: &Path) -> moqui_lib::error::Error {
            match EmbeddedCluster::start(dir, 20) {
                Err(err) => err,
                Ok((cluster, _driver)) => {
                    panic!("start succeeded for {}", cluster.data_dir().display());
                }
            }
        }

        fn prepared_dir() -> PathBuf {
            let dir = temp_cluster_dir();
            provide_libxml2_soname(&dir);
            dir
        }

        fn conn_drop_schema(driver: &PostgresDriver) {
            let mut conn = driver.connect_postgres().unwrap();
            conn.execute("DROP SCHEMA search CASCADE", &[]).unwrap();
        }

        struct KillGuard(u32);

        impl Drop for KillGuard {
            fn drop(&mut self) {
                if Path::new(&format!("/proc/{}", self.0)).exists() {
                    let _ = std::process::Command::new("kill")
                        .args(["-9", &self.0.to_string()])
                        .status();
                }
            }
        }

        fn kill_pid(pid: u32) {
            let status = std::process::Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status()
                .expect("kill");
            assert!(status.success(), "kill {pid}");
            for _ in 0..50 {
                if !Path::new(&format!("/proc/{pid}")).exists() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            panic!("postmaster {pid} still present after kill");
        }

        fn round_trip_scalars(conn: &mut PostgresConn) {
            conn.execute(
                "CREATE TABLE sample (
                    txt text,
                    flag boolean,
                    i2 int2,
                    i4 int4,
                    i8 int8,
                    f4 float4,
                    f8 float8,
                    n numeric,
                    ts timestamptz,
                    d date,
                    t time,
                    b bytea,
                    z int4
                )",
                &[],
            )
            .unwrap();
            let timestamp = kind("2024-07-15 01:14:00", "Timestamp");
            let date = kind("2024-07-15", "Date");
            let time = kind("01:14:00", "Time");
            let decimal = kind("3950.123456", "BigDecimal");
            let bytes = Value::Bytes(vec![1, 2, 3, 255]);
            conn.execute(
                "INSERT INTO sample (txt, flag, i2, i4, i8, f4, f8, n, ts, d, t, b, z) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                &[
                    Value::string("alpha"),
                    Value::Bool(true),
                    Value::I32(4),
                    Value::I32(7),
                    Value::I64(9),
                    Value::F64(1.5),
                    Value::F64(2.25),
                    decimal.clone(),
                    timestamp.clone(),
                    date.clone(),
                    time.clone(),
                    bytes.clone(),
                    Value::Null,
                ],
            )
            .unwrap();
            let rows = conn
                .query(
                    "SELECT txt, flag, i2, i4, i8, f4, f8, n, ts, d, t, b, z FROM sample",
                    &[],
                )
                .unwrap();
            assert_eq!(rows.len(), 1);
            let row = &rows[0];
            assert_eq!(row.get("txt"), Some(&Value::string("alpha")));
            assert_eq!(row.get("flag"), Some(&Value::Bool(true)));
            assert_eq!(row.get("i2"), Some(&Value::I32(4)));
            assert_eq!(row.get("i4"), Some(&Value::I32(7)));
            assert_eq!(row.get("i8"), Some(&Value::I64(9)));
            assert_eq!(row.get("f4"), Some(&Value::F64(1.5)));
            assert_eq!(row.get("f8"), Some(&Value::F64(2.25)));
            assert_eq!(row.get("n"), Some(&decimal));
            assert_eq!(row.get("ts"), Some(&timestamp));
            assert_eq!(row.get("d"), Some(&date));
            assert_eq!(row.get("t"), Some(&time));
            assert_eq!(row.get("b"), Some(&bytes));
            assert_eq!(row.get("z"), Some(&Value::Null));
        }

        fn savepoint_keeps_the_first_row(conn: &mut PostgresConn) {
            conn.execute("CREATE TABLE sp (n int)", &[]).unwrap();
            conn.begin().unwrap();
            conn.execute("INSERT INTO sp (n) VALUES (?)", &[Value::I32(1)]).unwrap();
            conn.savepoint("my sp").unwrap();
            conn.execute("INSERT INTO sp (n) VALUES (?)", &[Value::I32(2)]).unwrap();
            conn.rollback_to_savepoint("my sp").unwrap();
            conn.release_savepoint("my sp").unwrap();
            conn.commit().unwrap();
            let rows = conn.query("SELECT n FROM sp", &[]).unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].get("n"), Some(&Value::I32(1)));
        }

        fn stream_requires_a_transaction(conn: &mut PostgresConn) {
            let err = conn.open_stream("SELECT 1", &[]).unwrap_err();
            assert!(err.message().contains("transaction is required"), "{err}");
            assert!(!conn.stream_open());
        }

        fn stream_reads_every_inserted_row(conn: &mut PostgresConn) {
            conn.execute("CREATE TABLE streamed (n int)", &[]).unwrap();
            conn.begin().unwrap();
            for n in 1..=51 {
                conn.execute("INSERT INTO streamed (n) VALUES (?)", &[Value::I32(n)])
                    .unwrap();
            }
            conn.open_stream("SELECT n FROM streamed ORDER BY n", &[]).unwrap();
            assert!(conn.stream_open());
            for n in 1..=51 {
                let row = conn.next_row().unwrap().unwrap_or_else(|| panic!("missing row {n}"));
                assert_eq!(row.get("n"), Some(&Value::I32(n)));
            }
            assert!(conn.next_row().unwrap().is_none());
            assert!(conn.stream_open());
            conn.close_stream().unwrap();
            assert!(!conn.stream_open());
            conn.commit().unwrap();
        }

        fn show(conn: &mut PostgresConn, name: &str) -> String {
            let rows = conn.query_once(&format!("SHOW {name}"), &[]).unwrap();
            match &rows[0].columns[0].1 {
                Value::String(text) => text.clone(),
                other => panic!("show {name}: {other:?}"),
            }
        }

        fn kind(text: &str, java_type: &str) -> Value {
            let converted = basic_convert(&Value::string(text), java_type);
            assert!(
                !matches!(converted.value, Value::String(_)),
                "{text} did not convert to {java_type}: {:?}",
                converted.value
            );
            converted.value
        }

        fn provide_libxml2_soname(dir: &Path) {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            {
                let cache = ldconfig_cache();
                if cache.lines().any(|line| ldconfig_soname(line) == "libxml2.so.2") {
                    return;
                }
                let Some(host) = libxml2_16_path(&cache) else {
                    return;
                };
                compile_libxml2_shim(dir, &host);
            }
            #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
            let _ = dir;
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        fn ldconfig_cache() -> String {
            std::process::Command::new("ldconfig")
                .arg("-p")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                .unwrap_or_default()
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        fn ldconfig_soname(line: &str) -> &str {
            line.split_whitespace().next().unwrap_or("")
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        fn libxml2_16_path(cache: &str) -> Option<PathBuf> {
            for line in cache.lines() {
                if ldconfig_soname(line) == "libxml2.so.16" && line.contains("x86-64") {
                    let path = PathBuf::from(line.split("=>").nth(1)?.trim());
                    if path.is_file() {
                        return Some(path);
                    }
                }
            }
            None
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        fn compile_libxml2_shim(dir: &Path, host: &Path) {
            let shim_dir = dir.join("xmlshim");
            std::fs::create_dir_all(&shim_dir).unwrap();
            let assembly = shim_dir.join("shim.s");
            let map = shim_dir.join("ver.map");
            let shared = shim_dir.join("libxml2.so.2");
            std::fs::write(&assembly, shim_assembly()).unwrap();
            std::fs::write(&map, version_script()).unwrap();
            let output = std::process::Command::new("gcc")
                .args(["-shared", "-fPIC", "-o"])
                .arg(&shared)
                .arg(&assembly)
                .arg("-Wl,-soname,libxml2.so.2")
                .arg(format!("-Wl,--version-script={}", map.display()))
                .arg(host)
                .output()
                .expect("gcc");
            assert!(
                output.status.success(),
                "libxml2 shim: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let shim = shim_dir.display().to_string();
            let existing = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
            let library_path = if existing.split(':').any(|part| part == shim) {
                existing
            } else if existing.is_empty() {
                shim
            } else {
                format!("{shim}:{existing}")
            };
            // SAFETY: initdb and the postmaster inherit this. The test writes it before those spawn.
            unsafe { std::env::set_var("LD_LIBRARY_PATH", library_path) };
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        fn shim_assembly() -> String {
            let mut out = String::new();
            out.push_str(".text\n");
            for (index, (name, version)) in XML_SYMBOLS.iter().enumerate() {
                writeln!(
                    out,
                    ".globl shim_{index}\n.type shim_{index}, @function\nshim_{index}:\n    jmp {name}@PLT\n.size shim_{index}, .-shim_{index}\n.symver shim_{index}, {name}@{version}"
                )
                .unwrap();
            }
            out
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        fn version_script() -> String {
            let mut out = String::new();
            for (index, version) in XML_VERSIONS.iter().enumerate() {
                out.push_str(version);
                out.push_str(" {\n  global:\n");
                for (name, symbol_version) in XML_SYMBOLS {
                    if symbol_version == version {
                        writeln!(out, "    {name};").unwrap();
                    }
                }
                out.push_str("  local: *;\n}");
                if index == 0 {
                    out.push_str(";\n");
                } else {
                    writeln!(out, " {};", XML_VERSIONS[index - 1]).unwrap();
                }
            }
            out
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        const XML_VERSIONS: &[&str] = &[
            "LIBXML2_2.4.30",
            "LIBXML2_2.6.0",
            "LIBXML2_2.6.5",
            "LIBXML2_2.6.8",
            "LIBXML2_2.6.23",
        ];

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        const XML_SYMBOLS: &[(&str, &str)] = &[
            ("xmlAddChildList", "LIBXML2_2.4.30"),
            ("xmlBufferContent", "LIBXML2_2.4.30"),
            ("xmlBufferCreate", "LIBXML2_2.4.30"),
            ("xmlBufferFree", "LIBXML2_2.4.30"),
            ("xmlBufferLength", "LIBXML2_2.4.30"),
            ("xmlCharInRange", "LIBXML2_2.6.0"),
            ("xmlCheckVersion", "LIBXML2_2.4.30"),
            ("xmlCopyNode", "LIBXML2_2.4.30"),
            ("xmlCtxtReadDoc", "LIBXML2_2.6.0"),
            ("xmlCtxtReadMemory", "LIBXML2_2.6.0"),
            ("xmlDocSetRootElement", "LIBXML2_2.4.30"),
            ("xmlEncodeSpecialChars", "LIBXML2_2.4.30"),
            ("xmlFreeDoc", "LIBXML2_2.4.30"),
            ("xmlFreeNode", "LIBXML2_2.4.30"),
            ("xmlFreeParserCtxt", "LIBXML2_2.4.30"),
            ("xmlFreeTextWriter", "LIBXML2_2.6.0"),
            ("xmlGetExternalEntityLoader", "LIBXML2_2.4.30"),
            ("xmlGetUTF8Char", "LIBXML2_2.4.30"),
            ("xmlInitParser", "LIBXML2_2.4.30"),
            ("xmlKeepBlanksDefault", "LIBXML2_2.4.30"),
            ("xmlNewDoc", "LIBXML2_2.4.30"),
            ("xmlNewDocText", "LIBXML2_2.4.30"),
            ("xmlNewNode", "LIBXML2_2.4.30"),
            ("xmlNewParserCtxt", "LIBXML2_2.4.30"),
            ("xmlNewStringInputStream", "LIBXML2_2.4.30"),
            ("xmlNewTextWriterMemory", "LIBXML2_2.6.0"),
            ("xmlNodeDump", "LIBXML2_2.4.30"),
            ("xmlParseBalancedChunkMemory", "LIBXML2_2.4.30"),
            ("xmlParserPrintFileContext", "LIBXML2_2.4.30"),
            ("xmlSaveClose", "LIBXML2_2.6.8"),
            ("xmlSaveDoc", "LIBXML2_2.6.8"),
            ("xmlSaveToBuffer", "LIBXML2_2.6.23"),
            ("xmlSaveTree", "LIBXML2_2.6.8"),
            ("xmlSetExternalEntityLoader", "LIBXML2_2.4.30"),
            ("xmlSetGenericErrorFunc", "LIBXML2_2.4.30"),
            ("xmlSetStructuredErrorFunc", "LIBXML2_2.6.0"),
            ("xmlStrchr", "LIBXML2_2.4.30"),
            ("xmlStrcmp", "LIBXML2_2.4.30"),
            ("xmlStrdup", "LIBXML2_2.4.30"),
            ("xmlStrlen", "LIBXML2_2.4.30"),
            ("xmlStrncmp", "LIBXML2_2.4.30"),
            ("xmlStrstr", "LIBXML2_2.4.30"),
            ("xmlTextWriterEndElement", "LIBXML2_2.6.0"),
            ("xmlTextWriterStartElement", "LIBXML2_2.6.0"),
            ("xmlTextWriterWriteAttribute", "LIBXML2_2.6.0"),
            ("xmlTextWriterWriteBase64", "LIBXML2_2.6.0"),
            ("xmlTextWriterWriteBinHex", "LIBXML2_2.6.0"),
            ("xmlTextWriterWriteRaw", "LIBXML2_2.6.0"),
            ("xmlXPathCastBooleanToNumber", "LIBXML2_2.4.30"),
            ("xmlXPathCastBooleanToString", "LIBXML2_2.4.30"),
            ("xmlXPathCastNodeSetToString", "LIBXML2_2.4.30"),
            ("xmlXPathCastNodeToString", "LIBXML2_2.4.30"),
            ("xmlXPathCastNumberToString", "LIBXML2_2.4.30"),
            ("xmlXPathCompiledEval", "LIBXML2_2.4.30"),
            ("xmlXPathCtxtCompile", "LIBXML2_2.6.5"),
            ("xmlXPathFreeCompExpr", "LIBXML2_2.4.30"),
            ("xmlXPathFreeContext", "LIBXML2_2.4.30"),
            ("xmlXPathFreeObject", "LIBXML2_2.4.30"),
            ("xmlXPathNewContext", "LIBXML2_2.4.30"),
            ("xmlXPathRegisterNs", "LIBXML2_2.4.30"),
        ];

        fn temp_cluster_dir() -> PathBuf {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let name = format!("moqui-pg-{}-{nanos}", std::process::id());
            if Path::new("/tmp").is_dir() {
                PathBuf::from("/tmp").join(name)
            } else {
                std::env::temp_dir().join(name)
            }
        }

        fn postmaster_pid(data_dir: &Path) -> u32 {
            let text = std::fs::read_to_string(data_dir.join("postmaster.pid")).unwrap();
            text.lines().next().unwrap().trim().parse().unwrap()
        }
    }
}

#[cfg(all(feature = "embedded-postgres", feature = "postgres"))]
pub use cluster::{load_save, save_db, EmbeddedCluster};

#[cfg(not(all(feature = "embedded-postgres", feature = "postgres")))]
pub fn save_db(_dir: &std::path::Path, _dump: &std::path::Path) -> Result<(), moqui_lib::error::Error> {
    Err(moqui_lib::error::Error::new("embedded postgres is not in this build"))
}

#[cfg(not(all(feature = "embedded-postgres", feature = "postgres")))]
pub fn load_save(_dir: &std::path::Path, _dump: &std::path::Path) -> Result<(), moqui_lib::error::Error> {
    Err(moqui_lib::error::Error::new("embedded postgres is not in this build"))
}
