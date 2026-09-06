//! Mock management command
//!
//! Create, list, test, serve, and manage HTTP mock definitions.

use super::{MockAction, MockCommand};
use crate::ops;

/// Execute mock command
#[allow(clippy::large_futures)]
pub async fn execute(cmd: MockCommand) -> anyhow::Result<()> {
    match cmd.action {
        MockAction::Create {
            url,
            output,
            method,
            status,
            body,
            template,
            id,
            priority,
            collection,
            kind,
            interactive,
        } => ops::create_mock(ops::CreateMock {
            url,
            output,
            method,
            status,
            body,
            template,
            id,
            priority,
            collection,
            kind,
            interactive,
        }),
        MockAction::List {
            collection,
            verbose,
        } => ops::list_mocks(collection, verbose).await,
        MockAction::Show { mock_id } => ops::show_mock(&mock_id).await,
        MockAction::Test {
            method,
            path,
            query,
            headers,
            body,
            render,
            debug,
            mock_file,
            json,
        } => {
            ops::test_mock_match(ops::TestMockParams {
                method_str: method,
                path,
                query,
                headers,
                body,
                render,
                debug,
                mock_file,
                json,
            })
            .await
        }
        MockAction::Reload { dir } => ops::reload_mocks(dir).await,
        MockAction::Recordings { dir } => ops::list_recordings(dir),
        MockAction::Validate {
            path,
            format,
            stdin,
            file_format,
        } => ops::validate_mocks(path, &format, stdin, file_format).await,
        MockAction::Format {
            path,
            check,
            stdin,
            file_format,
        } => ops::format_mocks(path, check, stdin, file_format.as_deref()),
        MockAction::Convert {
            input,
            output,
            format,
            matching: _,
            interactive,
            preflight,
            redirects,
            browser_headers,
            absolute_urls,
            domains,
            static_assets,
            keep_sensitive_headers,
            keep_infra_headers,
            extract_bodies,
            body_threshold_kb,
            flatten_repeats,
            no_delays,
        } => {
            // An explicit `--format` wins; otherwise the extension the caller
            // typed is what they meant, and writing YAML into a file called
            // `.json` produces something nothing downstream will read back.
            let format = format.unwrap_or_else(|| ops::format_for(&output));
            ops::convert_har(ops::ConvertHarOptions {
                input,
                output,
                format,
                interactive,
                exclude_preflight: !preflight,
                exclude_redirects: !redirects,
                strip_browser_headers: !browser_headers,
                normalize_urls: !absolute_urls,
                allowed_domains: domains,
                exclude_static_assets: !static_assets,
                strip_sensitive_headers: !keep_sensitive_headers,
                strip_infrastructure_headers: !keep_infra_headers,
                extract_bodies,
                body_threshold_kb,
                sequence_repeated_requests: !flatten_repeats,
                preserve_latency: !no_delays,
            })
            .await
        }
        MockAction::Export {
            dir,
            output,
            collection,
        } => ops::export_to_har(dir, output, collection).await,
        MockAction::Consolidate {
            input,
            output,
            format,
            min_pattern,
            no_templates,
            generalize,
            verify,
            fail_under,
            verbose,
        } => {
            ops::consolidate_mocks(ops::ConsolidateArgs {
                input,
                output,
                format,
                min_pattern,
                enable_templates: !no_templates,
                generalize,
                verify,
                fail_under,
                verbose,
            })
            .await
        }
        MockAction::Serve {
            dir,
            port,
            host,
            mocks,
            mock_file,
            watch,
            cors,
            enable_render_endpoint,
            log_matches,
            verbose,
            open,
            no_explain,
        } => {
            ops::serve_mock_server(ops::MockServerConfig {
                port,
                host,
                mocks_dir: dir.or(mocks),
                mock_file,
                watch,
                cors,
                enable_render_endpoint,
                log_matches,
                verbose,
                open_browser: open,
                explain_unmatched: !no_explain,
            })
            .await
        }
    }
}
