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

//! Process edge. HTTP, PDF, SSH, and SSO are not implemented yet.
//! Database drivers live in [`db`].

pub mod db;

/// Process entry used by the `moqui` binary when it is started with no arguments.
pub fn run() -> &'static str {
    let _ = moqui_core::candidate();
    "moqui candidate build; server not implemented"
}

/// `pg_dump --format=custom` of the embedded `moqui` database at `dir`.
///
/// # Errors
///
/// Returns an error when the cluster does not start or `pg_dump` fails. A postmaster this call started is
/// stopped before it returns. A postmaster that was already live keeps running.
pub fn save_db(
    dir: impl AsRef<std::path::Path>,
    dump: impl AsRef<std::path::Path>,
) -> Result<(), moqui_lib::error::Error> {
    db::embedded::save_db(dir.as_ref(), dump.as_ref())
}

/// `pg_restore --format=custom` into the embedded `moqui` database at `dir`.
///
/// # Errors
///
/// Returns an error when the cluster does not start or `pg_restore` fails. A postmaster this call started is
/// stopped before it returns. A postmaster that was already live keeps running.
pub fn load_save(
    dir: impl AsRef<std::path::Path>,
    dump: impl AsRef<std::path::Path>,
) -> Result<(), moqui_lib::error::Error> {
    db::embedded::load_save(dir.as_ref(), dump.as_ref())
}

/// No arguments print [`run`]. `save-db` and `load-save` take a cluster directory and a dump file.
///
/// # Errors
///
/// Returns an error for any other argument list, or when a dump command fails.
pub fn dispatch<I, S>(args: I) -> Result<Option<&'static str>, moqui_lib::error::Error>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|arg| arg.as_ref().to_string()).collect();
    match args.as_slice() {
        [] => Ok(Some(run())),
        [command, dir, file] if command == "save-db" => {
            save_db(dir, file)?;
            Ok(None)
        }
        [command, dir, file] if command == "load-save" => {
            load_save(dir, file)?;
            Ok(None)
        }
        _ => Err(moqui_lib::error::Error::new(
            "usage: moqui [save-db <cluster-dir> <dump-file> | load-save <cluster-dir> <dump-file>]",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::dispatch;

    #[test]
    fn no_args_returns_the_candidate_line() {
        let line = dispatch(std::iter::empty::<&str>()).unwrap();
        assert_eq!(line, Some("moqui candidate build; server not implemented"));
    }

    #[test]
    fn unknown_command_names_the_dump_commands() {
        let err = dispatch(["lint"]).unwrap_err();
        assert!(err.message().contains("save-db"), "{err}");
        assert!(err.message().contains("load-save"), "{err}");
    }
}
