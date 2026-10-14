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

//! The edge runtime: one small Tokio runtime for the async clients the framework calls from blocking code, which are
//! the PDF browser, SSH and SFTP, the OIDC client, and the IPP print client. It is not the axum runtime, and an `ExecutionContext` thread
//! never enters it: [`block_on`] spawns the future here and waits on a plain channel, so it is safe from any thread,
//! including a `spawn_blocking` thread of another runtime.

use std::future::Future;
use std::sync::OnceLock;

use moqui_lib::error::Error;

/// The runtime, started on first use and kept for the life of the process.
///
/// # Panics
///
/// Panics when the operating system refuses the runtime's threads.
pub fn handle() -> tokio::runtime::Handle {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("moqui-edge")
                .enable_all()
                .build()
                .expect("the edge runtime starts")
        })
        .handle()
        .clone()
}

/// Run `future` on the edge runtime and block this thread until it finishes.
///
/// # Errors
///
/// Returns an error when the future panicked or the runtime dropped it.
pub fn block_on<F>(future: F) -> Result<F::Output, Error>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    handle().spawn(async move {
        let _ = tx.send(future.await);
    });
    rx.recv()
        .map_err(|_| Error::new("the edge runtime dropped the task before it finished"))
}
