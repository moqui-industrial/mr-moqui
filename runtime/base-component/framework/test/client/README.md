# The framework's client specs

HTTP and WebSocket proofs of the framework's security, run from outside the server by `moqui test-client`
([plans/ClientTestHarness.md](../../../../../plans/ClientTestHarness.md) 7). They port Moqui Java's pytest suite
(`framework/test/*.py` in the Java checkout, with `SECURITY_TESTS.md` and `SecurityTestSupport.groovy`); the catalog
below lists each of its tests.

```sh
moqui test-client framework --serve                  # starts the server suite.js declares, runs, stops it
./moq run                                            # or a server of your own, in one terminal,
./moq test-client framework                          # and the specs against http://localhost:8080
moqui test-client framework --spec SecurityAuthn,SecurityCors --server https://test.example.com --insecure
```

`--serve` starts `moqui test framework --spec ClientFixtures --db snapshot --serve` under
`conf/MoquiProductionConf.xml` (`suite.js`). Against another server:

- **The conf.** `SecurityCors` needs a server that allows no foreign origin, the production conf. On the dev conf
  (`webapp_allow_origins=*`) its `setupSpec` fails with "this server allows every origin (the dev conf)", as the Python
  suite's `require_production_conf` fails it, and the foreign-origin proof of `SecurityWebSocket` skips. The other
  specs pass on any conf.
- **The fixture users.** `../ClientFixtures.spec.js` makes `sec.view.only`, `sec.none.only` and `test.load` with
  Java's passwords, as `SecurityTestSupport.ensureUsers` did. They are not demo data. For a server `--serve` did not
  start, run it on that server's database: `moqui test framework --spec ClientFixtures --db existing`. Without them
  the features that need them skip, naming the command.
- **Users.** The other proofs log in as `john.doe`/`moqui` (`--param user=`, `--param password=`). None gives it a
  wrong password, since an account locks on its fourth failed login, and none logs it out over REST, which
  would end its other sessions (`UserAccount.hasLoggedOut`).

Each feature's name starts with its OWASP id, and a comment above it names the Python tests it ports, so
`grep -n test_<name> *.spec.js` finds the proof.

## Differences from Java

Recorded in [Parity-Diffs](../../../../../plans/Parity-Diffs.md) (P9-06 and the client test harness rows):

- A preflight answers 204, not 200.
- The session cookie is `moqui.session`, not `JSESSIONID`.
- The session-token failure is 401 "Session token is missing or is not valid" (`crates/moqui-core/src/screen/render.rs`),
  where Java says "Session token required" or "Session token does not match".
- An anonymous `/notws` upgrade is 401; Java accepts it and ignores the null user. So the foreign-origin proof uses a
  logged-in session, with the same session opening it from the server's own origin as the control.

Not a difference, but a choice of the suite: an account locks on its fourth failed login (`max-failures="3"`), as in
Java, and the suite must not lock the demo user, so no proof gives `john.doe` a wrong password. They use an unknown
user, and the wrong-password proofs wait for a dedicated `sec.*` user.

## Catalog

143 tests: 34 ported, 90 pending, 19 not applicable. Each test's status is `ported: <Spec> > <feature>`, `pending`
(with what it needs when that is more than a port), or `not applicable` with the reason, so grep counts them:
`grep -c '^| .test_.* | pending' README.md`. Patterns not built yet are multipart, HMAC (`/rest/sm` with the
`SEC_SMR_*` remotes as fixtures) and timing (the tarpit); further `sec.*` users join `ClientFixtures` as their tests
are ported. The Selenium smokes beside the suite (`openui_*`, `llm_admin_*`, `vue_sfc_*`) and the A2A interop script
are Java Assist checks and out of scope.

### test_a01_access.py

| Python test | Here |
| --- | --- |
| `test_tools_without_session_is_not_the_tool` | ported: SecurityAccess > A01 Tools without a session redirects to the login, not the tool |
| `test_rest_e1_without_credentials_is_401` | ported: SecurityAccess > A01 #path without credentials is 401 |
| `test_rest_s1_without_credentials_is_unauthorized` | ported: SecurityAccess > A01 #path without credentials is 401 |
| `test_removed_rest_api_key_is_not_found` | ported: SecurityAccess > A01 the removed REST transition #path is 404, not merely unauthenticated |
| `test_removed_rest_moqui_session_token_is_not_found` | ported: SecurityAccess > A01 the removed REST transition #path is 404, not merely unauthenticated |

### test_a01_more.py

| Python test | Here |
| --- | --- |
| `test_echopath_extra_path_is_html_escaped` | pending |
| `test_fop_without_login_is_not_a_pdf` | pending |
| `test_email_tracking_unknown_id_returns_png` | not applicable: /email (viewEmailMessage, the tracking pixel) is not ported |
| `test_status_from_loopback_is_json` | pending |
| `test_status_x_forwarded_for_does_not_grant_access_from_spoofed_loopback` | pending |
| `test_status_forwarded_for_header_does_not_hide_loopback` | pending |
| `test_login_form_action_is_login_path` | pending |
| `test_static_path_dotdot_does_not_escape` | ported: SecurityAccess > A01 a static path with .. does not serve screen XML |
| `test_view_only_cannot_run_service_run_over_http` | pending |
| `test_none_user_cannot_open_tools` | ported: SecurityAccess > A01 a user with no authorization cannot open Tools |
| `test_view_only_cannot_open_sql_runner` | ported: SecurityAccess > A01 a VIEW-only user cannot open the SQL Runner |
| `test_elastic_proxy_without_login_is_401` | pending: ports to /search (Java /elastic) and /dashboard (Java /kibana), permissions SearchRemote and DashboardRemote |
| `test_elastic_proxy_logged_in_without_permission_is_denied` | pending: ports to /search (Java /elastic) and /dashboard (Java /kibana), permissions SearchRemote and DashboardRemote |
| `test_kibana_proxy_without_login_is_401` | pending: ports to /search (Java /elastic) and /dashboard (Java /kibana), permissions SearchRemote and DashboardRemote |
| `test_unauthenticated_data_import_does_not_fetch_location` | pending |
| `test_apps_get_preferences_without_login_is_401` | ported: SecurityAccess > A01 getPreferences without a login is 401 with no preferences |
| `test_apps_qzsign_without_login_is_401` | pending |
| `test_htmlr_anonymous_error_does_not_include_stack` | pending: no /htmlr mapping here: decide when porting (a 404 proof, as for /h2) |
| `test_htmlr_tools_without_login_is_not_a_pdf` | pending: no /htmlr mapping here: decide when porting (a 404 proof, as for /h2) |
| `test_system_message_unknown_remote_is_rejected` | pending |
| `test_login_get_with_password_is_rejected` | pending |
| `test_web_inf_is_not_served` | pending |
| `test_groovysh_http_get_is_not_a_shell` | not applicable: /groovysh is excluded |
| `test_notws_http_get_is_not_a_websocket_handshake` | pending |
| `test_sql_runner_query_string_sql_is_not_executed` | pending |
| `test_menu_data_tools_without_login_is_not_the_menu` | pending |
| `test_elastic_proxy_basic_auth_without_permission_is_403` | pending: ports to /search (Java /elastic) and /dashboard (Java /kibana), permissions SearchRemote and DashboardRemote |
| `test_elastic_proxy_bad_basic_auth_is_401` | pending: ports to /search (Java /elastic) and /dashboard (Java /kibana), permissions SearchRemote and DashboardRemote |
| `test_elastic_proxy_api_key_header_garbage_is_401` | pending: ports to /search (Java /elastic) and /dashboard (Java /kibana), permissions SearchRemote and DashboardRemote |
| `test_elastic_proxy_basic_auth_with_permission_reaches_the_cluster` | pending: ports to /search (Java /elastic) and /dashboard (Java /kibana), permissions SearchRemote and DashboardRemote |
| `test_fop_filename_quote_stays_in_one_content_disposition` | pending |
| `test_datasnapshot_view_cannot_download` | pending |
| `test_datasnapshot_parent_segment_filename_is_not_found` | pending |
| `test_datasnapshot_parent_segment_delete_is_rejected` | pending |
| `test_datasnapshot_parent_segment_import_is_rejected` | pending |
| `test_elfinder_component_webroot_put_does_not_write_screen` | pending |
| `test_anonymous_tools_csv_is_not_a_data_dump` | pending |
| `test_view_only_tools_csv_is_not_a_privilege_escalation` | pending |
| `test_authorized_tools_csv_is_csv` | pending |
| `test_production_data_import_rejects_remote_location` | pending |
| `test_multipart_executable_upload_is_rejected` | pending: multipart |

### test_a01_redirect.py

| Python test | Here |
| --- | --- |
| `test_login_returnTo_does_not_redirect_offsite` | ported: SecurityAccess > A01 the login does not follow a returnTo off the site |
| `test_login_referer_does_not_redirect_offsite` | pending |
| `test_login_returnTo_does_not_follow_host_header` | pending |
| `test_login_returnTo_same_origin_path_is_allowed` | ported: SecurityAccess > A01 the login follows a returnTo on the site |

### test_a01_ws.py

| Python test | Here |
| --- | --- |
| `test_groovysh_anonymous_upgrade_is_not_a_shell` | not applicable: /groovysh is excluded |
| `test_groovysh_admin_can_eval` | not applicable: /groovysh is excluded |
| `test_groovysh_anonymous_after_admin_session_is_not_a_shell` | not applicable: /groovysh is excluded |
| `test_groovysh_view_only_is_not_a_shell` | not applicable: /groovysh is excluded |
| `test_notws_anonymous_upgrade_accepts_or_closes` | ported: SecurityWebSocket > A01 an anonymous /notws upgrade is 401 (replaced: the upgrade is 401 here; Java accepts it) |
| `test_websocket_evil_origin_is_rejected` | ported: SecurityWebSocket > A01 a /notws upgrade from a foreign Origin is refused, with a logged-in session (skips on the dev conf) |
| `test_groovysh_same_origin_admin_can_eval` | not applicable: /groovysh is excluded |

### test_a02_headers.py

| Python test | Here |
| --- | --- |
| `test_html_response_has_csp_and_frame_options` | ported: SecurityHeaders > A02 an HTML page has a CSP with frame-ancestors and form-action, X-Frame-Options and nosniff |
| `test_session_cookie_is_httponly` | ported: SecurityHeaders > A02 the session cookie is HttpOnly and SameSite=Lax |
| `test_cors_no_origin_header_has_no_allow_origin` | ported: SecurityHeaders > A02 a request with no Origin gets no Access-Control-Allow-Origin |
| `test_cors_unknown_origin_is_401` | ported: SecurityCors > A02 an unknown origin is 401 with no Access-Control-Allow-Origin (#method) |
| `test_cors_same_origin_returns_allow_origin` | ported: SecurityCors > A02 the server's own origin gets Access-Control-Allow-Origin with credentials |
| `test_cors_preflight_same_origin_is_allowed` | ported: SecurityCors > A02 a preflight from the server's own origin is allowed |
| `test_h2_console_is_not_mounted` | ported: SecurityHeaders > A02 no H2 console is mounted (there is no H2 console; the 404 proof is kept) |
| `test_session_cookie_samesite_lax` | ported: SecurityHeaders > A02 the session cookie is HttpOnly and SameSite=Lax |
| `test_login_html_is_not_publicly_cacheable` | ported: SecurityHeaders > A02 the login page is not publicly cacheable |
| `test_cors_preflight_unknown_origin_is_401` | ported: SecurityCors > A02 an unknown origin is 401 with no Access-Control-Allow-Origin (#method) |

### test_a05_injection.py

| Python test | Here |
| --- | --- |
| `test_rpc_set_preference_rejects_html_by_default` | not applicable: JSON-RPC (/rpc/json) was left out on purpose |
| `test_sql_looking_preference_is_stored_as_data` | pending |

### test_a07_authn.py

| Python test | Here |
| --- | --- |
| `test_rest_login_succeeds_without_csrf_token` | ported: SecurityAuthn > A07 the REST login needs no session token |
| `test_post_without_csrf_token_rejected_after_login` | pending |
| `test_post_with_csrf_token_is_not_a_csrf_error` | pending |
| `test_api_key_in_query_string_does_not_authenticate` | ported: SecurityAuthn > A07 an api_key #where does not log in |
| `test_post_with_csrf_token_executes_preference_round_trip` | ported: SecurityAuthn > A07 a POST runs with the session token and stores nothing without it (csrf: #csrf) |
| `test_post_without_csrf_does_not_store_preference` | ported: SecurityAuthn > A07 a POST runs with the session token and stores nothing without it (csrf: #csrf) |
| `test_api_key_header_with_garbage_does_not_authenticate` | ported: SecurityAuthn > A07 an api_key #where does not log in |
| `test_api_key_header_from_getLoginKey_authenticates` | pending: needs sec.key.http and its known login key as fixtures |
| `test_login_key_header_authenticates` | pending: needs sec.key.http and its known login key as fixtures |
| `test_api_key_in_json_body_authenticates` | not applicable: JSON-RPC (/rpc/json) was left out on purpose |
| `test_http_basic_on_screen_is_not_unauthenticated` | pending |
| `test_login_issues_a_new_session_id` | ported: SecurityAuthn > A07 the login form issues a new session id |

### test_a07_more.py

| Python test | Here |
| --- | --- |
| `test_rest_login_wrong_password_is_401` | pending: a wrong password needs a dedicated sec.* user (sec.lock.test): john.doe must not be locked |
| `test_rest_login_unknown_user_is_401` | ported: SecurityAuthn > A07 a REST login of an unknown user is 401 with the shared text |
| `test_sendOtp_without_preauth_is_rejected` | pending |
| `test_verifyOtp_without_preauth_does_not_log_in` | pending |
| `test_mfa_password_only_does_not_fully_log_in` | pending: needs sec.mfa.totp and its single-use factor as fixtures |
| `test_mfa_login_with_wrong_code_does_not_log_in` | pending: needs sec.mfa.totp and its single-use factor as fixtures |
| `test_mfa_login_with_code_succeeds` | pending: needs sec.mfa.totp and its single-use factor as fixtures |
| `test_create_initial_admin_http_fails_when_users_exist` | pending |
| `test_short_password_is_rejected_over_http` | pending: needs sec.all.only as a fixture |
| `test_failed_logins_lock_dedicated_user` | pending: needs sec.lock.test as a fixture |
| `test_login_json_response_omits_password_parameters` | pending |
| `test_login_json_failed_omits_password_parameters` | pending |
| `test_login_unknown_and_wrong_password_share_public_text` | pending: a wrong password needs a dedicated sec.* user (sec.lock.test): john.doe must not be locked |
| `test_password_reset_unknown_user_shows_shared_message` | pending |
| `test_password_reset_unknown_and_existing_share_public_text` | pending |
| `test_ip_restricted_user_cannot_login_from_loopback` | pending: needs sec.ip.v4 and sec.ip.loop as fixtures |
| `test_loopback_ip_allowed_user_can_login` | pending: needs sec.ip.v4 and sec.ip.loop as fixtures |

### test_a10_errors.py

| Python test | Here |
| --- | --- |
| `test_unauthorized_error_page_html_has_no_script_tag` | pending |
| `test_rest_unauth_html_error_has_no_script_or_stack` | pending |
| `test_internal_error_page_does_not_include_stack_trace_when_hit_directly` | pending |
| `test_rest_unauth_json_does_not_include_stack_frames` | ported: SecurityErrors > A10 a REST 401 in JSON has no stack frames |

### test_api_rest.py

| Python test | Here |
| --- | --- |
| `test_rpc_without_login_does_not_run_create_user` | not applicable: JSON-RPC (/rpc/json) was left out on purpose |
| `test_rpc_find_enumeration_without_allow_is_not_user_create` | not applicable: JSON-RPC (/rpc/json) was left out on purpose |
| `test_rest_userInfo_transition_is_gone` | ported: SecurityAccess > A01 the removed REST transition #path is 404, not merely unauthenticated |
| `test_rest_e1_user_account_without_entity_authz_is_not_200` | pending |
| `test_x_http_method_override_without_auth_is_401` | pending |
| `test_x_http_method_override_still_requires_csrf` | pending |
| `test_entity_rest_post_without_csrf_does_not_create` | pending |
| `test_rpc_json_without_csrf_does_not_reset_password` | not applicable: JSON-RPC (/rpc/json) was left out on purpose |
| `test_rest_e1_basic_auth_wrong_password_is_401` | ported: SecurityAuthn > A07 Basic on /rest/e1: #who (an unknown user, not john.doe with a wrong password) |
| `test_rest_e1_basic_auth_valid_is_not_unauthenticated` | ported: SecurityAuthn > A07 Basic on /rest/e1: #who |
| `test_system_message_hmac_missing_header_is_403` | pending: HMAC: needs the SEC_SMT_TEST and SEC_SMR_* remotes as fixtures, and signing in the client |
| `test_system_message_hmac_bad_signature_is_403` | pending: HMAC: needs the SEC_SMT_TEST and SEC_SMR_* remotes as fixtures, and signing in the client |
| `test_system_message_hmac_valid_does_not_require_login` | pending: HMAC: needs the SEC_SMT_TEST and SEC_SMR_* remotes as fixtures, and signing in the client |
| `test_system_message_hmac_timestamp_outside_window_is_403` | pending: HMAC: needs the SEC_SMT_TEST and SEC_SMR_* remotes as fixtures, and signing in the client |
| `test_system_message_hmac_timestamp_in_window_is_accepted` | pending: HMAC: needs the SEC_SMT_TEST and SEC_SMR_* remotes as fixtures, and signing in the client |
| `test_system_message_smat_none_accepts_anonymous` | pending: HMAC: needs the SEC_SMT_TEST and SEC_SMR_* remotes as fixtures, and signing in the client |
| `test_entity_rest_narrow_authz_allows_only_its_entity` | pending: needs sec.ent.view and sec.ent.all as fixtures |
| `test_entity_rest_catchall_authz_allows_other_entities` | pending: needs sec.ent.view and sec.ent.all as fixtures |
| `test_entity_sync_put_needs_all_authz` | not applicable: JSON-RPC (/rpc/json) was left out on purpose; the proof calls put#EntitySyncData over /rpc/json |
| `test_entity_sync_put_with_all_authz_stores` | not applicable: JSON-RPC (/rpc/json) was left out on purpose; the proof calls put#EntitySyncData over /rpc/json |
| `test_service_rest_moqui_api_view_can_read_users` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_service_rest_users_one_omits_password_hash_fields` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_entity_rest_user_account_omits_password_hash_fields` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_service_rest_moqui_api_view_cannot_update_users` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_service_rest_no_api_authz_cannot_read_users` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_service_rest_moqui_api_all_can_read_users` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_x_http_method_override_authz_uses_override_action` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_service_rest_users_patch_does_not_store_current_password` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_service_rest_users_patch_does_not_store_disabled` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_service_rest_users_patch_stores_full_name` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_entity_rest_does_not_create_user_login_key` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_email_server_host_not_writable_without_admin` | pending: needs sec.api.view, sec.api.all as fixtures |
| `test_entity_sync_put_does_not_store_user_group_permission` | not applicable: JSON-RPC (/rpc/json) was left out on purpose; the proof calls put#EntitySyncData over /rpc/json |
| `test_jsonrpc_array_body_returns_one_result_per_call` | not applicable: JSON-RPC (/rpc/json) was left out on purpose |
| `test_jsonrpc_array_body_over_limit_is_rejected` | not applicable: JSON-RPC (/rpc/json) was left out on purpose |
| `test_system_message_hmac_timestamp_replay_is_rejected` | pending: HMAC: needs the SEC_SMT_TEST and SEC_SMR_* remotes as fixtures, and signing in the client |
| `test_rest_schema_anonymous_is_401` | not applicable: /rest/entity.swagger answers 404 before any auth check (Swagger 2 is not built); the schema proof on /rest/entity.json is test_rest_schema_without_permission_is_403 |
| `test_rest_schema_without_permission_is_403` | pending: REST_SCHEMA on /rest/entity.json, granted to ADMIN by the seed data |
| `test_rest_schema_does_not_set_acao_wildcard` | not applicable: /rest/entity.swagger answers 404 before any auth check (Swagger 2 is not built); the schema proof on /rest/entity.json is test_rest_schema_without_permission_is_403 |

### test_z_tarpit.py

| Python test | Here |
| --- | --- |
| `test_screen_tarpit_returns_429_after_burst` | pending: timing: the tarpit, under the production conf; its own sec.tap.user fixture |
| `test_transition_tarpit_for_sec_tap_user_returns_429` | pending: timing: the tarpit, under the production conf; its own sec.tap.user fixture |
