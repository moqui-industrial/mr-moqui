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

//! Messages and errors for one execution context. Not process-global.

#[derive(Clone, Debug, Default)]
pub struct MessageFacade {
    messages: Vec<String>,
    errors: Vec<String>,
}

impl MessageFacade {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_message(&mut self, message: impl Into<String>) {
        let message = message.into();
        if message.is_empty() {
            return;
        }
        tracing::info!("{message}");
        self.messages.push(message);
    }

    pub fn add_error(&mut self, error: impl Into<String>) {
        let error = error.into();
        if error.is_empty() {
            return;
        }
        tracing::error!("{error}");
        self.errors.push(error);
    }

    pub fn messages(&self) -> &[String] {
        &self.messages
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.errors.clear();
    }
}
