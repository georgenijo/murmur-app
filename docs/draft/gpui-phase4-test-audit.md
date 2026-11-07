# Phase 4 Rust test migration audit

Compared `origin/gpui/26-ui-parity` with phase 27 (`9da3151c`) by extracting
`#[test]`, `#[tokio::test(...)]` and `#[gpui::test(...)]` function names from
all tracked Rust files outside vendored sources. React/TypeScript are excluded.
The base had 2,381 distinct names; phase 27 initially had 2,337, with 48 base
names absent. This follow-up restores 23 of those names and adds
`failed_native_dispatch_can_retry` for the surviving half of one mixed test.

## Restored to native paths

The headless target is now `app/crates/murmur-services/tests/headless_benchmark.rs`.
Its real inference test stays ignored. Only parsing tests run in normal suites.
All other restored tests use the actual service owners and native `Context`,
with synthetic ports, capture workers, temporary stores, and deferred event
transport. They create no GPUI/AppKit windows and perform no OS UI effects.

- `adapter_delivers_only_to_named_windows_and_native_consumers_without_main`
- `cancel_before_main_drain_drops_queued_waveform`
- `capture_publication_never_queries_windows_and_coalesces_a_busy_main_thread`
- `capture_revoked_during_visibility_query_is_dropped_before_delivery`
- `corpus_env_var_defaults_to_bundled`
- `file_cancel_ipc_completes_while_accept_owner_is_blocked`
- `headless_benchmark`
- `idle_after_a_timed_out_stop_clears_processing_for_the_current_recording`
- `models_env_var_defaults_to_installed_list_or_parses_csv`
- `out_path_is_required`
- `partial_lease_releases_on_worker_early_return_and_panic_without_clearing_prompt`
- `partial_lease_survives_cancelled_wrapper_until_worker_cleanup_finishes`
- `partial_pre_spawn_exits_drop_preview_before_lease_before_and_after_preparation`
- `preset_env_var_maps_to_expected_preset_or_errors`
- `production_subscriptions_and_producers_deliver_headlessly_with_synthetic_state`
- `quit_dispatch_remains_available_while_startup_reader_is_stalled`
- `replacement_before_main_drain_drops_old_waveform_and_delivers_new_generation`
- `revocation_waits_between_final_check_and_transport_or_callback_entry`
- `runtime_failure_bridge_emits_recovery_before_failure_and_records_one_failed_run`
- `stale_partial_lease_neither_releases_nor_wakes_a_new_owner`
- `standby_reactivation_cannot_revive_a_queued_waveform`
- `stop_before_main_drain_drops_queued_waveform`
- `terminal_teardown_retires_samples_before_it_can_block`

## Retired code and split coverage

These remaining names are deliberately absent because their tested implementation
was removed. They are not claimed as equivalent native fixture coverage.

### Deleted Tauri generated ACLs (3)

The generated IPC ACL/build capability files and Tauri command wrappers no longer
exist. Native sender authorization is covered by the native facade contracts.

- `generated_clipboard_acl_matches_build_configuration`
- `generated_shutdown_acl_is_feature_gated_and_main_only`
- `generated_updater_fixture_acl_is_qa_only_and_main_only`

### Deleted WebKit reload hook (one mixed test)

`failed_dispatch_can_retry_and_failed_reload_still_clears_only_crashed_interest`
combined two behaviors. Its dispatcher failure/retry assertions are restored as
`failed_native_dispatch_can_retry`; the Tauri web-content crash/reload hook and
renderer waveform subscriptions were deleted.

### Deleted Tauri overlay settle worker (1)

`placement_triggers_recheck_while_the_menu_bar_settles` tested the removed
`PlacementTrigger::settle_delays` schedule (Startup/DisplayChange/Settle included).
Native presentation owns placement; the surviving service enum only forwards
FrontmostApp/SpaceChange notifications and has no settle schedule.

### Deleted Tauri tray implementation (5)

The old 66×66 generated RGBA icon and `update_menu_label` formatter no longer
exist. Native tray presentation is injected via `NativePorts`; these tests do
not exercise that implementation.

- `tray_icon_center_pixel_is_opaque_white`
- `tray_icon_corner_pixel_is_transparent`
- `tray_icon_data_correct_size`
- `update_menu_label_rejects_unbounded_or_unsafe_versions`
- `update_menu_label_tracks_available_version`

### Removed optional shutdown QA receipt subsystem (14)

These tested the deleted `shutdown/qa_evidence.rs` module: its shutdown-test
identity gate, exclusive JSON evidence file, atomic receipt staging, fixed
100 ms evidence attempt and stderr panic containment. This was separate from
the production shutdown coordinator. Restoring the fixture would reintroduce a
retired subsystem, so these are classified as deleted-code tests (not merely
Tauri fixture tests). Production deadline/outcome and telemetry-drain tests
remain in `murmur-core`, `shutdown.rs`, and `telemetry.rs`, including
`telemetry_success_preserves_content_writer_outcome`,
`shutdown_barrier_is_bounded_and_accounts_for_stalled_startup_output`, and
`failed_telemetry_write_degrades_committed_writers_and_suppresses_restart`.

- `atomic_publication_never_exposes_partial_json_or_clobbers_receipt`
- `atomic_publication_sync_failures_preserve_exclusive_complete_receipts`
- `evidence_panics_still_run_the_terminal_continuation_once`
- `identity_gate_precedes_all_directory_access`
- `one_receipt_per_process_with_closed_content_schema`
- `owner_failure_and_preserved_writes_have_distinct_codes`
- `panicking_payload_destructor_cannot_skip_terminal_action`
- `phase_deadlines_and_late_completions_do_not_erase_failure_codes`
- `publication_refuses_a_racing_receipt_and_preserves_unowned_staging`
- `slow_fsync_cannot_hold_terminal_action_or_start_a_second_attempt`
- `successful_bounded_write_is_recorded_once_with_content_free_status`
- `terminal_receipt_sync_failures_do_not_claim_success_or_overwrite_json`
- `terminal_timeout_during_publication_sync_has_no_staging_cleanup_left`
- `unwritable_destination_and_broken_stderr_cannot_skip_terminal_action`

The related `terminal_evidence_survives_both_sealed_and_delayed_writer_paths`
(`telemetry.rs`) also used that removed QA `Evidence` writer and receipt schema.
It is retired with that module; the production telemetry barrier tests above
remain active.
