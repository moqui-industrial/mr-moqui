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

//! What an html error page knows of the failure: the `errorThrowable` of `error/*.xml`. The Java page read the
//! exception; this holds the facts that the pages used from it.

use crate::context::{Error, ErrorKind};

/// The artifact a denied request was for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactInfo {
    pub name: String,
    pub action: String,
    pub artifact_type: String,
}

impl ArtifactInfo {
    #[must_use]
    pub fn action_description(&self) -> String {
        match self.action.as_str() {
            "view" => "View",
            "create" => "Create",
            "update" => "Update",
            "delete" => "Delete",
            "all" => "All Actions",
            other => other,
        }
        .to_string()
    }

    #[must_use]
    pub fn type_description(&self) -> String {
        match self.artifact_type.as_str() {
            "AT_XML_SCREEN" => "Screen",
            "AT_XML_SCREEN_TRANS" => "Screen Transition",
            "AT_XML_SCREEN_CONTENT" => "Screen Content",
            "AT_SERVICE" => "Service",
            "AT_ENTITY" => "Entity",
            "AT_REST_PATH" => "REST Path",
            other => other,
        }
        .to_string()
    }
}

#[derive(Clone, Debug)]
pub struct ErrorInfo {
    pub status: u16,
    pub message: String,
    /// The message chain, one line each.
    pub trace: Vec<String>,
    pub artifact: Option<ArtifactInfo>,
    pub path_from_last_screen: String,
    pub full_path: Vec<String>,
}

impl ErrorInfo {
    #[must_use]
    pub fn new(status: u16, error: &Error, request_path: &str) -> Self {
        let message = error.message().to_string();
        let artifact = if error.kind() == ErrorKind::Unauthorized {
            parse_denied(&message)
        } else {
            None
        };
        let path_from_last_screen = message
            .strip_prefix("screen path not found: ")
            .and_then(|rest| rest.split(" under ").next())
            .unwrap_or_default()
            .to_string();
        Self {
            status,
            trace: message.lines().map(str::to_string).collect(),
            message,
            artifact,
            path_from_last_screen,
            full_path: request_path
                .split('/')
                .filter(|part| !part.is_empty())
                .map(str::to_string)
                .collect(),
        }
    }
}

/// `User x is not authorized for AT_XML_SCREEN [location] action view`.
fn parse_denied(message: &str) -> Option<ArtifactInfo> {
    let rest = message.split_once(" is not authorized for ")?.1;
    let (artifact_type, rest) = rest.split_once(" [")?;
    let (name, rest) = rest.split_once("] action ")?;
    let action = rest.split_whitespace().next().unwrap_or("").to_string();
    Some(ArtifactInfo {
        name: name.to_string(),
        action,
        artifact_type: artifact_type.to_string(),
    })
}

/// The page that shows a failure with this status and the error type of its `error-screen` entry.
#[must_use]
pub fn error_screen(status: u16) -> (&'static str, &'static str) {
    match status {
        401 => ("unauthorized", "error/Unauthorized"),
        403 => ("forbidden", "error/Forbidden"),
        404 => ("not-found", "error/NotFound"),
        429 => ("too-many", "error/TooMany"),
        _ => ("internal-error", "error/InternalError"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_denied_message_names_its_artifact() {
        let error = Error::of_kind(
            ErrorKind::Unauthorized,
            "User admin is not authorized for AT_XML_SCREEN [component://x/screen/A.xml] action view because of a deny record",
        );
        let info = ErrorInfo::new(403, &error, "/apps/A");
        let artifact = info.artifact.unwrap();
        assert_eq!(artifact.name, "component://x/screen/A.xml");
        assert_eq!(artifact.action_description(), "View");
        assert_eq!(artifact.type_description(), "Screen");
        assert_eq!(info.full_path, vec!["apps", "A"]);
    }

    #[test]
    fn a_missing_path_is_the_name_that_was_not_found() {
        let error = Error::of_kind(
            ErrorKind::NotFound,
            "screen path not found: Nope under component://x/y.xml",
        );
        let info = ErrorInfo::new(404, &error, "/Nope");
        assert_eq!(info.path_from_last_screen, "Nope");
        assert!(info.artifact.is_none());
    }

    #[test]
    fn statuses_choose_a_page() {
        assert_eq!(error_screen(401).1, "error/Unauthorized");
        assert_eq!(error_screen(404).0, "not-found");
        assert_eq!(error_screen(500).1, "error/InternalError");
        assert_eq!(error_screen(503).1, "error/InternalError");
    }
}
