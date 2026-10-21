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

//! Function registry for `type="rust"` services. Each function registers under its Rust path, down to the function
//! name (`moqui_core::service::mail::send_template`). The service element's `location` is that path.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use moqui_lib::error::Error as LibError;
use moqui_lib::object::Value;

use crate::entity::EntityFacade;

/// A Rust service body. The service name stays on the definition; the pointer is the `location`.
pub(crate) type RustServiceFn =
    fn(&BTreeMap<String, Value>, &EntityFacade) -> Result<BTreeMap<String, Value>, LibError>;

/// Register `$function` under `module_path!() + "::" + its name`. A second insert of that path is an error.
macro_rules! rust_service {
    ($map:expr, $function:ident) => {{
        let location: &'static str = concat!(module_path!(), "::", stringify!($function));
        debug_assert_eq!(location, ::std::any::type_name_of_val(&$function));
        if $map.insert(location, $function).is_some() {
            Err(format!("duplicate rust service {location}"))
        } else {
            Ok(())
        }
    }};
}

static REGISTRY: OnceLock<BTreeMap<&'static str, RustServiceFn>> = OnceLock::new();

/// The Rust service locations of the features a build may leave out (docs/Packaging.md), by path prefix; the first
/// prefix that matches decides. A service at one of these locations still loads in a build without the feature, and a
/// call reports that the feature is not built into this host.
const FEATURE_LOCATIONS: &[(&str, &str, bool)] = &[
    ("moqui_core::service::git::", "git", cfg!(feature = "git")),
    ("moqui_core::service::images::", "image", cfg!(feature = "image")),
    ("moqui_core::service::spreadsheet::", "xlsx", cfg!(feature = "xlsx")),
    (
        "moqui_core::service::mail::send_template",
        "smtp",
        cfg!(feature = "smtp"),
    ),
    (
        "moqui_core::service::mail::send_message",
        "smtp",
        cfg!(feature = "smtp"),
    ),
    ("moqui_core::service::mail::test_server", "smtp", cfg!(feature = "smtp")),
    (
        "moqui_core::service::mail::push_remote_changes",
        "smtp",
        cfg!(feature = "smtp"),
    ),
    // the app's contacts hook, with or without mail built in
    ("moqui_core::service::mail::filter_contact_addresses", "", true),
    ("moqui_core::service::mail::", "imap", cfg!(feature = "imap")),
];

/// The feature `location` belongs to when this build leaves that feature out.
pub(crate) fn missing_feature(location: &str) -> Option<&'static str> {
    FEATURE_LOCATIONS
        .iter()
        .find(|(prefix, _, _)| location.starts_with(prefix))
        .and_then(|(_, feature, built)| (!built).then_some(*feature))
}

/// The function registered at `location`, once every module has registered.
pub(crate) fn get(location: &str) -> Option<RustServiceFn> {
    registry().get(location).copied()
}

fn registry() -> &'static BTreeMap<&'static str, RustServiceFn> {
    REGISTRY.get_or_init(build)
}

fn build() -> BTreeMap<&'static str, RustServiceFn> {
    let mut map = BTreeMap::new();
    let mut errors = Vec::new();
    for result in [
        super::backup::register(&mut map),
        super::content::register(&mut map),
        super::mail::register(&mut map),
        super::system_sync::register(&mut map),
        super::elfinder::register(&mut map),
        super::sso::register(&mut map),
        super::tools::register(&mut map),
        #[cfg(feature = "image")]
        super::images::register(&mut map),
        #[cfg(feature = "xlsx")]
        super::spreadsheet::register(&mut map),
        super::shell::register(&mut map),
        super::user_keys::register(&mut map),
        super::dav::register(&mut map),
        super::collab::register(&mut map),
        super::chat::register(&mut map),
        super::chat_link::register(&mut map),
        #[cfg(feature = "git")]
        super::git::register(&mut map),
    ] {
        if let Err(error) = result {
            errors.push(error);
        }
    }
    assert!(errors.is_empty(), "rust service registry: {}", errors.join("; "));
    map
}

#[cfg(test)]
mod tests {
    #[test]
    fn registry_keys_are_the_function_paths() {
        let map = super::build();
        let mut keys: Vec<&str> = map.keys().copied().collect();
        keys.sort_unstable();
        let mut expected = vec![
            "moqui_core::service::backup::backup_embedded_database",
            "moqui_core::service::chat::check_record_visible",
            "moqui_core::service::chat::push_signal",
            "moqui_core::service::chat_link::extract_link_urls",
            "moqui_core::service::chat_link::fetch_link_page",
            "moqui_core::service::collab::get_document",
            "moqui_core::service::collab::save_document",
            "moqui_core::service::content::gc_content_blobs",
            "moqui_core::service::content::migrate_db_resource_blobs",
            "moqui_core::service::dav::get_sync_addresses",
            "moqui_core::service::dav::serve_request",
            "moqui_core::service::elfinder::run_command",
            "moqui_core::service::images::convert_image",
            "moqui_core::service::images::get_rich_text_image",
            "moqui_core::service::images::image_reader_file_suffixes",
            "moqui_core::service::images::image_reader_mime_types",
            "moqui_core::service::images::image_writer_file_suffixes",
            "moqui_core::service::images::image_writer_mime_types",
            "moqui_core::service::images::upload_rich_text_image",
            "moqui_core::service::images::write_image_service",
            "moqui_core::service::mail::complete_oauth_sign_in",
            "moqui_core::service::mail::discover_server",
            "moqui_core::service::mail::export_message",
            "moqui_core::service::mail::filter_contact_addresses",
            "moqui_core::service::mail::find_oauth_provider",
            "moqui_core::service::mail::get_oauth_providers",
            "moqui_core::service::mail::get_oauth_status",
            "moqui_core::service::mail::import_messages",
            "moqui_core::service::mail::poll_server",
            "moqui_core::service::mail::push_remote_changes",
            "moqui_core::service::mail::revoke_oauth_sign_in",
            "moqui_core::service::mail::revoke_oauth_token",
            "moqui_core::service::mail::send_message",
            "moqui_core::service::mail::send_template",
            "moqui_core::service::mail::start_oauth_sign_in",
            "moqui_core::service::mail::sync_account",
            "moqui_core::service::mail::sync_due_accounts",
            "moqui_core::service::mail::test_server",
            "moqui_core::service::shell::get_shell_counts",
            "moqui_core::service::shell::publish_shell_counts",
            "moqui_core::service::spreadsheet::load_spreadsheet_entity",
            "moqui_core::service::spreadsheet::read_spreadsheet",
            "moqui_core::service::sso::handle_callback",
            "moqui_core::service::sso::login_user",
            "moqui_core::service::sso::logout_user",
            "moqui_core::service::system_sync::entity_sync::get_data",
            "moqui_core::service::system_sync::entity_sync::include_list",
            "moqui_core::service::system_sync::entity_sync::internal_run",
            "moqui_core::service::system_sync::entity_sync::put_data",
            "moqui_core::service::system_sync::entity_sync::run_sync",
            "moqui_core::service::system_sync::message::send_rest",
            "moqui_core::service::system_sync::sftp::poll",
            "moqui_core::service::system_sync::sftp::send",
            "moqui_core::service::tools::cancel_print_job",
            "moqui_core::service::tools::get_print_job_details",
            "moqui_core::service::tools::get_server_printers",
            "moqui_core::service::tools::hold_print_job",
            "moqui_core::service::tools::print_document",
            "moqui_core::service::tools::print_resource_document",
            "moqui_core::service::tools::print_screen_document",
            "moqui_core::service::tools::release_print_job",
            "moqui_core::service::tools::rewrite_encrypted_fields",
            "moqui_core::service::tools::send_document_notifications",
            "moqui_core::service::tools::send_print_job_internal",
            "moqui_core::service::user_keys::create_access_token",
            "moqui_core::service::user_keys::create_app_password",
            "moqui_core::service::user_keys::create_ssh_key",
            "moqui_core::service::user_keys::delete_ssh_key",
            "moqui_core::service::user_keys::get_app_passwords",
            "moqui_core::service::user_keys::revoke_access_token",
            "moqui_core::service::user_keys::revoke_app_password",
        ];
        if cfg!(feature = "git") {
            expected.extend([
                "moqui_core::service::git::commit_files",
                "moqui_core::service::git::export_repository",
                "moqui_core::service::git::fetch_all_repositories",
                "moqui_core::service::git::fetch_repository",
                "moqui_core::service::git::fsck_repository",
                "moqui_core::service::git::gc_repositories",
                "moqui_core::service::git::get_blob",
                "moqui_core::service::git::get_clone_urls",
                "moqui_core::service::git::get_commit",
                "moqui_core::service::git::get_commit_diff",
                "moqui_core::service::git::get_log",
                "moqui_core::service::git::get_refs",
                "moqui_core::service::git::import_repository",
                "moqui_core::service::git::init_repository",
                "moqui_core::service::git::process_pending_updates",
                "moqui_core::service::git::process_update",
                "moqui_core::service::git::setup_lfs",
            ]);
            expected.sort_unstable();
        }
        // A feature this build leaves out registers nothing; its locations resolve through missing_feature.
        expected.retain(|location| super::missing_feature(location).is_none());
        assert_eq!(keys, expected);
    }
}
