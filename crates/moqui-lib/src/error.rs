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

//! Plain error for `moqui-lib`. Artifact exceptions stay in `moqui-core`.

use std::fmt;

/// Where a parsed node or a parse failure sits in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: Option<String>,
    pub line: u32,
    pub column: u32,
}

/// Failure from a lib utility. Library paths return this instead of panicking.
#[derive(Debug)]
pub struct Error {
    message: String,
    location: Option<SourceLocation>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            location: None,
            source: None,
        }
    }

    pub fn at(message: impl Into<String>, file: Option<String>, line: u32, column: u32) -> Self {
        Self {
            message: message.into(),
            location: Some(SourceLocation { file, line, column }),
            source: None,
        }
    }

    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn location(&self) -> Option<&SourceLocation> {
        self.location.as_ref()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(loc) = &self.location {
            write!(f, " (")?;
            if let Some(file) = &loc.file {
                write!(f, "{file}:")?;
            }
            write!(f, "{}:{})", loc.line, loc.column)?;
        }
        if let Some(source) = &self.source {
            write!(f, ": {source}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|err| err.as_ref() as &(dyn std::error::Error + 'static))
    }
}
