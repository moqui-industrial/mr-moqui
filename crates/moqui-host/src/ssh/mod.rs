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

//! SSH on `russh` and SFTP on `russh-sftp` (`plans/archive/Ssh.md`): the SFTP client behind `moqui_core::sftp`, and the
//! built-in server with its SFTP subsystem and admin shell.

pub mod client;
pub mod cluster;
pub mod server;
mod sftp_fs;
mod shell;

use std::path::Path;
use std::sync::{Arc, Weak};

use moqui_core::context::{Error as CoreError, ExecutionContext, ExecutionContextFactory};

/// Run `job` on a request worker, in a fresh context with authorization off until the job logs a user in, and wait
/// without blocking the async runtime. The SSH tasks hold the factory weakly, so a server that stopped is not kept
/// alive by them, and its database clients are never dropped on a runtime thread.
pub(crate) async fn on_pool<T, F>(factory: &Weak<ExecutionContextFactory>, job: F) -> Result<T, CoreError>
where
    T: Send + 'static,
    F: FnOnce(&ExecutionContextFactory) -> Result<T, CoreError> + Send + 'static,
{
    let factory = Weak::clone(factory);
    tokio::task::spawn_blocking(move || {
        let rx = {
            let factory = factory
                .upgrade()
                .ok_or_else(|| CoreError::new("the server is stopping"))?;
            let owner = Arc::clone(&factory);
            let inner = Arc::clone(&factory);
            factory.request_pool().try_submit(owner, move || {
                let _authz = ExecutionContext::with_current(ExecutionContext::disable_authz);
                job(&inner)
            })?
        };
        rx.recv().map_err(|_| CoreError::new("request worker stopped"))?
    })
    .await
    .map_err(|_| CoreError::new("request worker panicked"))?
}

/// Install the SFTP client into core. First-use host keys go in `runtime/conf/ssh_known_hosts`.
pub fn install_client(runtime_dir: &Path) {
    let known_hosts = runtime_dir.join("conf").join("ssh_known_hosts");
    moqui_core::sftp::install_sftp_connector(Arc::new(client::RusshConnector::new(known_hosts)));
}
