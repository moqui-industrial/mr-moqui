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

/// What a caller can act on. The web layer maps these to HTTP statuses; see `web::status_for`.
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
    /// Input that does not pass a service's parameter checks. The checks that failed are validation errors in the
    /// message facade.
    Validation,
    /// A request the client got wrong, such as a missing or ambiguous name in a REST path.
    BadRequest,
    /// The transaction was marked rollback-only, so it rolled back or takes no more statements.
    RollbackOnly,
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

/// The artifact an `Unauthorized` error refused: what the `error/Forbidden` page names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeniedArtifact {
    pub artifact_type: String,
    pub name: String,
    pub action: String,
}

pub struct Error {
    message: String,
    kind: ErrorKind,
    denied: Option<Box<DeniedArtifact>>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ErrorKind::General,
            denied: None,
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

    #[must_use]
    pub fn with_denied(mut self, denied: DeniedArtifact) -> Self {
        self.denied = Some(Box::new(denied));
        self
    }

    /// The refused artifact of the first context error in the source chain that names one.
    pub fn denied(&self) -> Option<&DeniedArtifact> {
        let mut current: Option<&(dyn std::error::Error + 'static)> = Some(self);
        while let Some(found) = current {
            if let Some(denied) = found.downcast_ref::<Error>().and_then(|err| err.denied.as_deref()) {
                return Some(denied);
            }
            current = found.source();
        }
        None
    }

    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A context error that crossed a lib-typed closure comes back as it went in. Any other lib error keeps the kind of a
/// context error in its source chain.
impl From<moqui_lib::error::Error> for Error {
    fn from(err: moqui_lib::error::Error) -> Self {
        let carried = match err.into_carried() {
            Ok(carried) => carried,
            Err(err) => {
                return Self {
                    message: err.message().to_string(),
                    kind: ErrorKind::of(&err),
                    denied: None,
                    source: Some(Box::new(err)),
                }
            }
        };
        match carried.downcast::<Error>() {
            Ok(err) => *err,
            Err(other) => Self {
                message: other.to_string(),
                kind: ErrorKind::of(other.as_ref()),
                denied: None,
                source: Some(other),
            },
        }
    }
}

/// Carried whole, so its kind, source, and refused artifact come back on the way out of lib-typed code.
impl From<Error> for moqui_lib::error::Error {
    fn from(err: Error) -> Self {
        moqui_lib::error::Error::carry(err.message.clone(), err)
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
