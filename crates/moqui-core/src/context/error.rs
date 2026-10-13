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

//! Boot and facade errors. Display is the message only, never a secret value.

use std::fmt;

/// What a caller can act on. The web layer maps these to 401, 403, and 429.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ErrorKind {
    #[default]
    General,
    /// The call needs a logged-in user.
    AuthenticationRequired,
    /// The user is logged in and not allowed.
    Unauthorized,
    /// Too many hits on an artifact.
    Tarpit,
    /// There is no such resource.
    NotFound,
    /// The resource exists but not for this method.
    MethodNotAllowed,
    /// The service cannot take the work now, such as a full request queue.
    Unavailable,
}

impl ErrorKind {
    /// The kind of the first context [`Error`] in an error's source chain, or `General`.
    pub fn of(err: &(dyn std::error::Error + 'static)) -> Self {
        let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
        while let Some(found) = current {
            if let Some(context) = found.downcast_ref::<Error>() {
                return context.kind;
            }
            current = found.source();
        }
        Self::General
    }
}

pub struct Error {
    message: String,
    kind: ErrorKind,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::General,
            source: None,
        }
    }

    pub fn of_kind(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            ..Self::new(message)
        }
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<moqui_lib::error::Error> for Error {
    fn from(err: moqui_lib::error::Error) -> Self {
        let message = err.message().to_string();
        Self::new(message).with_source(err)
    }
}

impl From<Error> for moqui_lib::error::Error {
    fn from(err: Error) -> Self {
        moqui_lib::error::Error::new(err.message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(source) = &self.source {
            write!(f, ": {source}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|err| err.as_ref() as &(dyn std::error::Error + 'static))
    }
}
