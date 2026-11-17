//! Tests for C6.2 (ordered checks, short-circuit) and C6.3 (the matrix).

use super::*;

type Mutate = fn(&mut Occupancy);

fn refused(code: RefusalCode) -> Admission {
    Admission::RefuseWith(code)
}

/// One C6.2 check: its name, the mutation that makes it fire, and the outcome
/// it must produce.
struct Case {
    name: &'static str,
    fail: Mutate,
    expect: Admission,
}

fn case(name: &'static str, fail: Mutate, expect: Admission) -> Case {
    Case { name, fail, expect }
}

/// Asserts, for one request:
/// 1. the implemented check order equals the C6.2 order written in `cases`;
/// 2. a quiet occupancy passes every check and evaluates all of them;
/// 3. for every check `i`, an occupancy where checks `i..` all fire returns
///    check `i`'s outcome and **evaluates exactly `cases[..=i]`**, so no later
///    check ran (first failure returns);
/// 4. each check fires on its own with the same outcome.
fn assert_order(request: Request, base: fn() -> Occupancy, cases: Vec<Case>) {
    let names: Vec<_> = cases.iter().map(|c| c.name).collect();
    assert_eq!(check_order(request), names, "{request:?}: C6.2 order");

    let quiet = admit_traced(request, &base());
    assert!(
        matches!(quiet.admission, Admission::Admit(_)),
        "{request:?}: quiet occupancy must admit, got {:?}",
        quiet.admission
    );
    assert_eq!(
        quiet.evaluated, names,
        "{request:?}: admit evaluates every check"
    );

    for (i, case) in cases.iter().enumerate() {
        let mut cumulative = base();
        for later in &cases[i..] {
            (later.fail)(&mut cumulative);
        }
        let result = admit_traced(request, &cumulative);
        assert_eq!(
            result.admission, case.expect,
            "{request:?}: check #{i} `{}` fires first",
            case.name
        );
        assert_eq!(
            result.evaluated,
            names[..=i].to_vec(),
            "{request:?}: checks after `{}` must not be evaluated",
            case.name
        );

        let mut alone = base();
        (case.fail)(&mut alone);
        let solo = admit_traced(request, &alone);
        assert_eq!(
            solo.admission, case.expect,
            "{request:?}: `{}` fires on its own",
            case.name
        );
        assert_eq!(solo.evaluated, names[..=i].to_vec());
    }
}

fn quiet() -> Occupancy {
    Occupancy::default()
}

#[test]
fn dictation_start_order() {
    assert_order(
        Request::DictationStart,
        quiet,
        vec![
            case(
                "app_disabled",
                |o| o.app_disabled = true,
                refused(RefusalCode::AppDisabled),
            ),
            case(
                "smart_auto_with_fixed_device",
                |o| o.fixed_and_smart_auto = true,
                refused(RefusalCode::SmartAutoWithFixedDevice),
            ),
            case(
                "relaunch_committed",
                |o| o.relaunch_committed = true,
                refused(RefusalCode::BusyRestarting),
            ),
            case(
                "meeting_blocks_asr",
                |o| o.meeting_inference_active = true,
                refused(RefusalCode::BusyMeeting),
            ),
            case(
                "file_transcribing",
                |o| o.file_transcribing = true,
                refused(RefusalCode::BusyTranscribingFile),
            ),
            case(
                "corpus_active",
                |o| o.corpus_active = true,
                refused(RefusalCode::BusyRecordingCorpus),
            ),
            case(
                "benchmark_running",
                |o| o.benchmark_running = true,
                refused(RefusalCode::BusyBenchmarking),
            ),
            case(
                "status",
                |o| o.status = Status::Recording,
                refused(RefusalCode::AlreadyRecording),
            ),
            case(
                "cleanup_pending",
                |o| o.cleanup_pending = true,
                refused(RefusalCode::AudioRecoveringCleanupPending),
            ),
            case(
                "smart_auto_selection",
                |o| o.smart_auto = SmartAutoObservation::Requested(None),
                Admission::NeedObservation(ObsKind::SmartAutoSelection),
            ),
        ],
    );
}

#[test]
fn dictation_start_status_and_smart_auto_arms() {
    let at = |status| Occupancy { status, ..quiet() };
    let expect = [
        (Status::Starting, RefusalCode::AlreadyStarting),
        (Status::Recording, RefusalCode::AlreadyRecording),
        (Status::Recovering, RefusalCode::AudioRecovering),
        (Status::Processing, RefusalCode::AlreadyProcessing),
    ];
    for (status, code) in expect {
        assert_eq!(admit(Request::DictationStart, &at(status)), refused(code));
    }
    // Blockers precede status (5506 before 5513).
    let both = Occupancy {
        status: Status::Recording,
        benchmark_running: true,
        ..quiet()
    };
    assert_eq!(
        admit(Request::DictationStart, &both),
        refused(RefusalCode::BusyBenchmarking)
    );
    // `Err` from smart-auto resolution refuses with that text and burns no id.
    let failed = Occupancy {
        smart_auto: SmartAutoObservation::Requested(Some(Err("no device".to_string()))),
        ..quiet()
    };
    assert_eq!(
        admit(Request::DictationStart, &failed),
        refused(RefusalCode::SmartAutoFailed("no device".to_string()))
    );
    let resolved = Occupancy {
        smart_auto: SmartAutoObservation::Requested(Some(Ok(()))),
        ..quiet()
    };
    assert!(matches!(
        admit(Request::DictationStart, &resolved),
        Admission::Admit(Claims {
            claim: ClaimKind::DictationTransition,
            ..
        })
    ));
}

#[test]
fn dictation_start_preview_disposition() {
    let preview = Occupancy {
        preview_active: true,
        ..quiet()
    };
    assert_eq!(
        admit(Request::DictationStart, &preview),
        Admission::Admit(Claims {
            claim: ClaimKind::DictationTransition,
            preview: PreviewDisposition::StopThenRetry
        })
    );
    let kept = Occupancy {
        handoff_candidate: Some(4),
        ..preview
    };
    assert_eq!(
        admit(Request::DictationStart, &kept),
        Admission::Admit(Claims {
            claim: ClaimKind::DictationTransition,
            preview: PreviewDisposition::KeptForHandoff(PreviewId(4))
        })
    );
}

#[test]
fn dictation_prepare_order() {
    assert_order(
        Request::DictationPrepare,
        quiet,
        vec![
            case(
                "start_blocker",
                |o| o.status = Status::Processing,
                refused(RefusalCode::DictationBusy),
            ),
            case(
                "preview_active",
                |o| o.preview_active = true,
                refused(RefusalCode::PreviewActive),
            ),
            case(
                "audio_active",
                |o| o.audio_active = true,
                refused(RefusalCode::MicrophoneBusy),
            ),
            case(
                "cleanup_pending",
                |o| o.cleanup_pending = true,
                refused(RefusalCode::CleanupPending),
            ),
        ],
    );
}

#[test]
fn start_blocker_covers_every_blocker_for_prepare_and_relaunch() {
    let blockers: [Mutate; 6] = [
        |o| o.meeting_active = true,
        |o| o.file_transcribing = true,
        |o| o.corpus_active = true,
        |o| o.benchmark_running = true,
        |o| o.status = Status::Starting,
        |o| o.relaunch_committed = true,
    ];
    for (index, blocker) in blockers.iter().enumerate() {
        let mut o = quiet();
        blocker(&mut o);
        assert_eq!(
            admit(Request::DictationPrepare, &o),
            refused(RefusalCode::DictationBusy)
        );
        // The relaunch gate excludes its own commitment (the last blocker).
        let expected = if index == 5 {
            Admission::Admit(Claims::of(ClaimKind::None))
        } else {
            refused(RefusalCode::DictationBusy)
        };
        assert_eq!(
            admit(Request::RelaunchGate, &o),
            expected,
            "blocker {index}"
        );
    }
}

#[test]
fn key_down_capture_order() {
    assert_order(
        Request::KeyDownCapture,
        quiet,
        vec![
            case(
                "listener_matches",
                |o| o.key_down.listener_matches = false,
                refused(RefusalCode::KeyDownListenerChanged),
            ),
            case(
                "keyboard_processing",
                |o| o.keyboard_processing = true,
                refused(RefusalCode::KeyDownProcessingSkipped),
            ),
            case(
                "app_disabled",
                |o| o.app_disabled = true,
                refused(RefusalCode::KeyDownDisabled),
            ),
            case(
                "gesture_active",
                |o| o.key_down.gesture_active = false,
                refused(RefusalCode::KeyDownGestureEnded),
            ),
            case(
                "microphone_permission",
                |o| o.key_down.microphone_granted = false,
                refused(RefusalCode::KeyDownPermissionNotGranted),
            ),
            case(
                "start_blocker",
                |o| o.file_transcribing = true,
                refused(RefusalCode::DictationBusy),
            ),
            case(
                "preview_active",
                |o| o.preview_active = true,
                refused(RefusalCode::PreviewActive),
            ),
            case(
                "audio_active",
                |o| o.audio_active = true,
                refused(RefusalCode::MicrophoneBusy),
            ),
            case(
                "cleanup_pending",
                |o| o.cleanup_pending = true,
                refused(RefusalCode::CleanupPending),
            ),
        ],
    );
}

#[test]
fn key_down_takes_over_only_the_handoff_candidate() {
    let preview = Occupancy {
        preview_active: true,
        handoff_candidate: Some(11),
        ..quiet()
    };
    assert_eq!(
        admit(Request::KeyDownCapture, &preview),
        Admission::Takeover(PreviewId(11))
    );
    // The takeover is the first failing check: later checks are not evaluated.
    let busy_behind = Occupancy {
        audio_active: true,
        cleanup_pending: true,
        ..preview.clone()
    };
    let result = admit_traced(Request::KeyDownCapture, &busy_behind);
    assert_eq!(result.admission, Admission::Takeover(PreviewId(11)));
    assert_eq!(result.evaluated.last(), Some(&"preview_active"));
    // First-tap preparation never takes a preview over.
    assert_eq!(
        admit(Request::DictationPrepare, &preview),
        refused(RefusalCode::PreviewActive)
    );
}

#[test]
fn relaunch_gate_order() {
    assert_order(
        Request::RelaunchGate,
        quiet,
        vec![
            case(
                "activity_blocker",
                |o| o.status = Status::Recovering,
                refused(RefusalCode::DictationBusy),
            ),
            case(
                "preview_active",
                |o| o.preview_active = true,
                refused(RefusalCode::PreviewActive),
            ),
            case(
                "audio_active",
                |o| o.audio_active = true,
                refused(RefusalCode::MicrophoneBusy),
            ),
            case(
                "cleanup_pending",
                |o| o.cleanup_pending = true,
                refused(RefusalCode::CleanupPending),
            ),
        ],
    );
}

#[test]
fn file_transcription_pre_claim_order() {
    assert_order(
        Request::FileTranscriptionPreClaim,
        quiet,
        vec![
            case(
                "preview_stop",
                |o| o.preview_stop = Some(Err("stop failed".to_string())),
                refused(RefusalCode::Text("stop failed".to_string())),
            ),
            case(
                "meeting_blocks_asr",
                |o| o.meeting_active = true,
                refused(RefusalCode::FileWaitForMeeting),
            ),
            case(
                "file_claim",
                |o| o.file_claim_prior = Some(true),
                refused(RefusalCode::FileAlreadyTranscribing),
            ),
        ],
    );
}

#[test]
fn file_transcription_claim_reads_the_swap_result_when_present() {
    // The shell already swapped: the prior value decides, the flag is not read.
    let swapped_clear = Occupancy {
        file_transcribing: true,
        file_claim_prior: Some(false),
        ..quiet()
    };
    assert!(matches!(
        admit(Request::FileTranscriptionPreClaim, &swapped_clear),
        Admission::Admit(_)
    ));
    // A visible preview is stopped first, never handed off.
    let preview = Occupancy {
        preview_active: true,
        preview_stop: Some(Ok(())),
        ..quiet()
    };
    assert_eq!(
        admit(Request::FileTranscriptionPreClaim, &preview),
        Admission::Admit(Claims {
            claim: ClaimKind::FileTranscription,
            preview: PreviewDisposition::StoppedFirst
        })
    );
}

#[test]
fn file_transcription_post_claim_order() {
    assert_order(
        Request::FileTranscriptionPostClaim,
        quiet,
        vec![
            case(
                "benchmark_running",
                |o| o.benchmark_running = true,
                refused(RefusalCode::FileWaitForBenchmark),
            ),
            case(
                "corpus_active",
                |o| o.corpus_active = true,
                refused(RefusalCode::FileFinishCorpus),
            ),
            case(
                "status",
                |o| o.status = Status::Recording,
                refused(RefusalCode::FileLiveAudioActive),
            ),
        ],
    );
}

#[test]
fn meeting_pre_claim_order() {
    assert_order(
        Request::MeetingPreClaim,
        quiet,
        vec![
            case(
                "relaunch_committed",
                |o| o.relaunch_committed = true,
                refused(RefusalCode::MeetingRestarting),
            ),
            case(
                "mic_startup_benchmark_active",
                |o| o.mic_startup_benchmark_active = true,
                refused(RefusalCode::MeetingMicStartupCheck),
            ),
            case(
                "corpus_active",
                |o| o.corpus_active = true,
                refused(RefusalCode::MeetingFinishCorpus),
            ),
            case(
                "meeting_active",
                |o| o.meeting_active = true,
                refused(RefusalCode::MeetingAlreadyActive),
            ),
            case(
                "meeting_inference_active",
                |o| o.meeting_inference_active = true,
                refused(RefusalCode::MeetingRecovering),
            ),
            case(
                "audio_active",
                |o| o.audio_active = true,
                refused(RefusalCode::MeetingStopMicrophone),
            ),
            case(
                "file_transcribing",
                |o| o.file_transcribing = true,
                refused(RefusalCode::MeetingWaitForFile),
            ),
            case(
                "benchmark_running",
                |o| o.benchmark_running = true,
                refused(RefusalCode::MeetingWaitForBenchmark),
            ),
            case(
                "status",
                |o| o.status = Status::Processing,
                refused(RefusalCode::MeetingWaitForDictation),
            ),
        ],
    );
}

#[test]
fn meeting_post_claim_order() {
    assert_order(
        Request::MeetingPostClaim,
        quiet,
        vec![case(
            "relaunch_recheck",
            |o| o.relaunch_recheck = Some(true),
            refused(RefusalCode::MeetingRestarting),
        )],
    );
    let clear = Occupancy {
        relaunch_recheck: Some(false),
        ..quiet()
    };
    assert!(matches!(
        admit(Request::MeetingPostClaim, &clear),
        Admission::Admit(_)
    ));
}

#[test]
fn preview_start_order() {
    assert_order(
        Request::PreviewStart,
        quiet,
        vec![
            case(
                "standby",
                |o| o.standby = true,
                Admission::WaitThen(Retry::RetireStandby),
            ),
            case(
                "start_revoked",
                |o| o.preview_start_revoked = true,
                refused(RefusalCode::PreviewDictationBusy),
            ),
            case(
                "status",
                |o| o.status = Status::Starting,
                refused(RefusalCode::PreviewDictationBusy),
            ),
            case(
                "meeting_blocks_asr",
                |o| o.meeting_active = true,
                refused(RefusalCode::PreviewMeeting),
            ),
            case(
                "benchmark_running",
                |o| o.benchmark_running = true,
                refused(RefusalCode::PreviewBenchmark),
            ),
            case(
                "corpus_active",
                |o| o.corpus_active = true,
                refused(RefusalCode::PreviewCorpus),
            ),
            case(
                "app_disabled",
                |o| o.app_disabled = true,
                refused(RefusalCode::EnableMurmurBeforeMicTest),
            ),
            case(
                "preview_active",
                |o| o.preview_active = true,
                refused(RefusalCode::PreviewAlreadyActive),
            ),
            case(
                "unconfirmed_key_down",
                |o| o.unconfirmed_key_down = true,
                Admission::WaitThen(Retry::RetireKeyDownCapture),
            ),
            case(
                "audio_active",
                |o| o.audio_active = true,
                refused(RefusalCode::MicrophoneStillInUse),
            ),
            case(
                "preview_still_connecting",
                |o| o.preview_still_connecting = Some(false),
                refused(RefusalCode::PreviewNoLongerConnecting),
            ),
        ],
    );
}

#[test]
fn preview_start_rechecks_after_the_retire() {
    // After the shell retires the key-down capture the microphone is free: the
    // second `admit` with a fresh occupancy admits.
    let retired = quiet();
    assert_eq!(
        admit(Request::PreviewStart, &retired),
        Admission::Admit(Claims::of(ClaimKind::PreviewGeneration))
    );
    // If something still holds the microphone after the retire, it refuses.
    let still = Occupancy {
        audio_active: true,
        ..quiet()
    };
    assert_eq!(
        admit(Request::PreviewStart, &still),
        refused(RefusalCode::MicrophoneStillInUse)
    );
}

#[test]
fn mic_startup_benchmark_pre_claim_order() {
    assert_order(
        Request::MicStartupBenchmarkPreClaim,
        quiet,
        vec![
            case(
                "standby",
                |o| o.standby = true,
                Admission::WaitThen(Retry::RetireStandby),
            ),
            case(
                "meeting_blocks_asr",
                |o| o.meeting_active = true,
                refused(RefusalCode::MicBenchMeeting),
            ),
            case(
                "status",
                |o| o.status = Status::Recording,
                refused(RefusalCode::MicBenchStopRecording),
            ),
            case(
                "file_transcribing",
                |o| o.file_transcribing = true,
                refused(RefusalCode::WaitForFileTranscription),
            ),
            case(
                "corpus_active",
                |o| o.corpus_active = true,
                refused(RefusalCode::MicBenchCorpus),
            ),
            case(
                "audio_active",
                |o| o.audio_active = true,
                refused(RefusalCode::MicrophoneStillInUse),
            ),
            case(
                "app_disabled",
                |o| o.app_disabled = true,
                refused(RefusalCode::EnableMurmurBeforeMicTest),
            ),
            case(
                "coordinator",
                |o| o.benchmark_running = true,
                refused(RefusalCode::MicBenchAlreadyRunning),
            ),
        ],
    );
}

#[test]
fn mic_startup_benchmark_post_claim_order() {
    assert_order(
        Request::MicStartupBenchmarkPostClaim,
        quiet,
        vec![case(
            "second_claim",
            |o| o.second_claim = Some(false),
            refused(RefusalCode::MicBenchAlreadyRunning),
        )],
    );
}

#[test]
fn coordinator_refusals_pick_their_message() {
    let model = Occupancy {
        shared_backend_change: true,
        ..quiet()
    };
    assert_eq!(
        admit(Request::MicStartupBenchmarkPreClaim, &model),
        refused(RefusalCode::MicBenchWaitForModelPrep)
    );
    assert_eq!(
        admit(Request::Benchmark, &model),
        refused(RefusalCode::BenchWaitForModelPrep)
    );
    let running = Occupancy {
        benchmark_running: true,
        shared_backend_change: true,
        ..quiet()
    };
    assert_eq!(
        admit(Request::Benchmark, &running),
        refused(RefusalCode::BenchAlreadyRunning)
    );
    // The coordinator mutex's own answer counts even with no flag set.
    let coordinator = Occupancy {
        coordinator_idle: Some(false),
        ..quiet()
    };
    assert_eq!(
        admit(Request::Benchmark, &coordinator),
        refused(RefusalCode::BenchWaitForModelPrep)
    );
}

#[test]
fn benchmark_order() {
    assert_order(
        Request::Benchmark,
        quiet,
        vec![
            case(
                "meeting_blocks_asr",
                |o| o.meeting_active = true,
                refused(RefusalCode::BenchMeeting),
            ),
            case(
                "status",
                |o| o.status = Status::Recovering,
                refused(RefusalCode::BenchStopRecording),
            ),
            case(
                "file_transcribing",
                |o| o.file_transcribing = true,
                refused(RefusalCode::WaitForFileTranscription),
            ),
            case(
                "corpus_active",
                |o| o.corpus_active = true,
                refused(RefusalCode::BenchCorpus),
            ),
            case(
                "coordinator",
                |o| o.shared_backend_change = true,
                refused(RefusalCode::BenchWaitForModelPrep),
            ),
        ],
    );
}

#[test]
fn corpus_order() {
    assert_order(
        Request::Corpus,
        quiet,
        vec![
            case(
                "meeting_blocks_asr",
                |o| o.meeting_active = true,
                refused(RefusalCode::CorpusMeeting),
            ),
            case(
                "status",
                |o| o.status = Status::Processing,
                refused(RefusalCode::CorpusDictation),
            ),
            case(
                "file_transcribing",
                |o| o.file_transcribing = true,
                refused(RefusalCode::CorpusFile),
            ),
            case(
                "benchmark_running",
                |o| o.benchmark_running = true,
                refused(RefusalCode::CorpusBenchmark),
            ),
            case(
                "corpus_claim",
                |o| o.corpus_active = true,
                refused(RefusalCode::CorpusAlreadyActive),
            ),
        ],
    );
}

#[test]
fn shared_backend_change_order() {
    assert_order(
        Request::SharedBackendChange,
        quiet,
        vec![case(
            "coordinator",
            |o| o.coordinator_idle = Some(false),
            refused(RefusalCode::SharedBackendBusy),
        )],
    );
    // It reads nothing about dictation: admitted while dictation records.
    let recording = Occupancy {
        status: Status::Recording,
        audio_active: true,
        ..quiet()
    };
    assert_eq!(
        admit(Request::SharedBackendChange, &recording),
        Admission::Admit(Claims::of(ClaimKind::SharedBackendChange))
    );
}

#[test]
fn model_preparation_order() {
    fn eligible() -> Occupancy {
        Occupancy {
            status: Status::Recording,
            model_matches: true,
            ..Occupancy::default()
        }
    }
    assert_order(
        Request::ModelPreparation,
        eligible,
        vec![case(
            "eligible",
            |o| o.model_matches = false,
            refused(RefusalCode::ModelPreparationNotEligible),
        )],
    );
    // Recording-start variant: Starting and Recording only.
    for (status, ok) in [
        (Status::Idle, false),
        (Status::Starting, true),
        (Status::Recording, true),
        (Status::Recovering, false),
        (Status::Processing, false),
    ] {
        let o = Occupancy {
            status,
            ..eligible()
        };
        assert_eq!(
            matches!(admit(Request::ModelPreparation, &o), Admission::Admit(_)),
            ok,
            "{status:?}"
        );
    }
    // Idle warm-up variant: Idle only.
    for status in Status::ALL {
        let o = Occupancy {
            status,
            model_prep_variant: ModelPrepVariant::IdleWarm,
            ..eligible()
        };
        assert_eq!(
            matches!(admit(Request::ModelPreparation, &o), Admission::Admit(_)),
            status == Status::Idle,
            "{status:?}"
        );
    }
}

#[test]
fn model_removal_order() {
    assert_order(
        Request::ModelRemoval,
        quiet,
        vec![
            case(
                "transition_lock",
                |o| o.transition_lock_busy = true,
                refused(RefusalCode::ModelRemovalTransitionBusy),
            ),
            case(
                "dictation_lock",
                |o| o.dictation_lock_busy = true,
                refused(RefusalCode::ModelRemovalDictationBusy),
            ),
            case(
                "occupied",
                |o| o.audio_active = true,
                refused(RefusalCode::ModelRemovalOccupied),
            ),
            case(
                "coordinator",
                |o| o.shared_backend_change = true,
                refused(RefusalCode::ModelRemovalOperationBusy),
            ),
        ],
    );
}

#[test]
fn model_removal_refuses_every_occupant_the_paragraph_names() {
    let occupants: [(&str, Mutate); 8] = [
        ("Starting", |o| o.status = Status::Starting),
        ("Processing", |o| o.status = Status::Processing),
        ("file", |o| o.file_transcribing = true),
        ("meeting", |o| o.meeting_active = true),
        ("meeting inference", |o| o.meeting_inference_active = true),
        ("retiring capture", |o| {
            o.cleanup_pending = true;
            o.audio_active = true;
        }),
        ("standby", |o| {
            o.standby = true;
            o.audio_active = true;
        }),
        ("unconfirmed key-down", |o| {
            o.unconfirmed_key_down = true;
            o.audio_active = true;
        }),
    ];
    for (name, occupant) in occupants {
        let mut o = quiet();
        occupant(&mut o);
        assert_eq!(
            admit(Request::ModelRemoval, &o),
            refused(RefusalCode::ModelRemovalOccupied),
            "{name}"
        );
    }
    // It reads nothing else: relaunch, preview, corpus, cleanup alone are `A`.
    for occupant in [
        (|o: &mut Occupancy| o.relaunch_committed = true) as Mutate,
        |o| o.preview_active = true,
        |o| o.corpus_active = true,
        |o| o.cleanup_pending = true,
    ] {
        let mut o = quiet();
        occupant(&mut o);
        assert!(matches!(
            admit(Request::ModelRemoval, &o),
            Admission::Admit(_)
        ));
    }
    // A benchmark, a mic-startup benchmark or another model change holds the claim.
    for occupant in [
        (|o: &mut Occupancy| o.benchmark_running = true) as Mutate,
        |o| o.shared_backend_change = true,
    ] {
        let mut o = quiet();
        occupant(&mut o);
        assert_eq!(
            admit(Request::ModelRemoval, &o),
            refused(RefusalCode::ModelRemovalOperationBusy)
        );
    }
}

#[test]
fn every_request_has_a_non_empty_check_list_and_unique_names() {
    for request in Request::ALL {
        let names = check_order(request);
        assert!(!names.is_empty(), "{request:?}");
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            names.len(),
            "{request:?}: duplicate check name"
        );
    }
}

// ---------------------------------------------------------------------------
// C6.3 cross-request matrix
// ---------------------------------------------------------------------------

/// Occupant columns of C6.3, in order.
const COLUMNS: [&str; 15] = [
    "dictation Starting/Recording",
    "dictation Processing",
    "dictation Recovering",
    "retiring capture",
    "draining pipeline",
    "file transcription",
    "meeting / meeting inference",
    "benchmark running",
    "shared backend change",
    "standby",
    "preview active",
    "unconfirmed key-down",
    "mic-startup check",
    "corpus",
    "relaunch committed",
];

/// The occupant of column `col`, as the shell would gather it (see the
/// [`Occupancy`] invariants: a retiring/standby/unconfirmed capture has
/// `audio_active`; a running mic-startup check holds the coordinator's
/// `Benchmark` activity).
fn occupant(col: usize) -> Occupancy {
    let mut o = Occupancy::default();
    match col {
        0 => o.status = Status::Recording,
        1 => o.status = Status::Processing,
        2 => o.status = Status::Recovering,
        3 => {
            o.cleanup_pending = true;
            o.audio_active = true;
        }
        4 => o.dictation_pipeline_draining = true,
        5 => o.file_transcribing = true,
        6 => o.meeting_active = true,
        7 => o.benchmark_running = true,
        8 => o.shared_backend_change = true,
        9 => {
            o.standby = true;
            o.audio_active = true;
        }
        10 => {
            o.preview_active = true;
            o.handoff_candidate = Some(9);
            o.preview_stop = Some(Ok(()));
        }
        11 => {
            o.unconfirmed_key_down = true;
            o.audio_active = true;
        }
        12 => {
            o.mic_startup_benchmark_active = true;
            o.benchmark_running = true;
        }
        13 => o.corpus_active = true,
        14 => o.relaunch_committed = true,
        _ => unreachable!(),
    }
    o
}

/// The row's admission: multi-phase requests run their phases in code order.
fn admit_row(row: &str, o: &Occupancy) -> Admission {
    match row {
        "DictationStart" => admit(Request::DictationStart, o),
        "DictationPrepare" => admit(Request::DictationPrepare, o),
        "KeyDownCapture" => admit(Request::KeyDownCapture, o),
        "RelaunchGate" => admit(Request::RelaunchGate, o),
        "FileTranscription" => match admit(Request::FileTranscriptionPreClaim, o) {
            Admission::Admit(pre) => match admit(Request::FileTranscriptionPostClaim, o) {
                Admission::Admit(post) => {
                    // The post-claim phase carries no claim of its own (C6.1a).
                    assert_eq!(post.claim, ClaimKind::None);
                    Admission::Admit(pre)
                }
                other => other,
            },
            other => other,
        },
        "Meeting" => match admit(Request::MeetingPreClaim, o) {
            Admission::Admit(_) => admit(Request::MeetingPostClaim, o),
            other => other,
        },
        "PreviewStart" => admit(Request::PreviewStart, o),
        "MicStartupBenchmark" => admit(Request::MicStartupBenchmarkPreClaim, o),
        "Benchmark" => admit(Request::Benchmark, o),
        "Corpus" => admit(Request::Corpus, o),
        "SharedBackendChange" => admit(Request::SharedBackendChange, o),
        other => panic!("unknown matrix row {other}"),
    }
}

/// `R` refuse, `A` allow, `K` takeover/preempt, `W` wait-then-retry.
fn classify(admission: &Admission) -> char {
    match admission {
        Admission::Admit(claims) if claims.preview != PreviewDisposition::NotPresent => 'K',
        Admission::Admit(_) => 'A',
        Admission::Takeover(_) => 'K',
        Admission::RefuseWith(_) => 'R',
        Admission::WaitThen(_) => 'W',
        Admission::NeedObservation(_) => 'N',
    }
}

/// C6.3 as written: rows × the 15 columns of [`COLUMNS`]. `S` (supervisor
/// decides) and `-` (not read, preserved as `A`) are both "`admit` allows".
const MATRIX: [(&str, &str); 11] = [
    ("DictationStart", "RRRRARRRASKSRRR"),
    ("DictationPrepare", "RRRRARRRARRRRRR"),
    ("KeyDownCapture", "RRRRARRRARKRRRR"),
    ("RelaunchGate", "RRRR-RRR-RRRRR-"),
    ("FileTranscription", "RRRA-RRR--K--R-"),
    ("Meeting", "RRRR-RRR-R-RRRR"),
    ("PreviewStart", "RRRR--RR-WRW-R-"),
    ("MicStartupBenchmark", "RRRR-RRRRW--RR-"),
    ("Benchmark", "RRRA-RRRR---RR-"),
    ("Corpus", "RRRA-RRR-----R-"),
    ("SharedBackendChange", "AAAAAAARRAAARAA"),
];

/// Cells where the code refuses although C6.3 prints `–` or says otherwise,
/// found by reading the code (see the README "Contradictions"). The code wins.
const CODE_WINS: [(&str, usize, char, &str); 4] = [
    (
        "FileTranscription",
        12,
        'R',
        "recording.rs transcribe_file reads benchmark.is_running(), which a mic-startup check holds",
    ),
    (
        "PreviewStart",
        12,
        'R',
        "microphone_preview.rs reads benchmark.is_running(), which a mic-startup check holds",
    ),
    (
        "Corpus",
        12,
        'R',
        "corpus.rs reads benchmark.is_running(), which a mic-startup check holds",
    ),
    (
        "MicStartupBenchmark",
        11,
        'R',
        "microphone_startup_benchmark.rs reads is_audio_active(), true for an unconfirmed key-down capture",
    ),
];

fn expected_cell(row: &str, col: usize, printed: char) -> char {
    if let Some((_, _, code, _)) = CODE_WINS.iter().find(|(r, c, _, _)| *r == row && *c == col) {
        return *code;
    }
    match printed {
        // `S`: the supervisor decides; `-`: the code makes no check.
        'S' | '-' => 'A',
        other => other,
    }
}

#[test]
fn c6_3_matrix_every_cell() {
    let mut cells = 0;
    for (row, printed) in MATRIX {
        assert_eq!(printed.chars().count(), COLUMNS.len(), "{row}: row width");
        for (col, printed_cell) in printed.chars().enumerate() {
            let expected = expected_cell(row, col, printed_cell);
            let admission = admit_row(row, &occupant(col));
            assert_eq!(
                classify(&admission),
                expected,
                "{row} × {} (C6.3 prints `{printed_cell}`): got {admission:?}",
                COLUMNS[col]
            );
            cells += 1;
        }
    }
    assert_eq!(cells, 11 * 15);
}

#[test]
fn code_wins_overrides_really_differ_from_the_printed_matrix() {
    for (row, col, code, _) in CODE_WINS {
        let printed = MATRIX
            .iter()
            .find(|(r, _)| *r == row)
            .and_then(|(_, cells)| cells.chars().nth(col))
            .expect("row exists");
        let printed_class = if matches!(printed, 'S' | '-') {
            'A'
        } else {
            printed
        };
        assert_ne!(printed_class, code, "{row} col {col} no longer differs");
    }
}

#[test]
fn matrix_refusal_codes_match_the_literals_where_the_matrix_names_them() {
    let code = |row: &str, col: usize| match admit_row(row, &occupant(col)) {
        Admission::RefuseWith(code) => code,
        other => panic!("{row} col {col}: expected a refusal, got {other:?}"),
    };
    assert_eq!(code("DictationStart", 0), RefusalCode::AlreadyRecording);
    assert_eq!(code("DictationStart", 1), RefusalCode::AlreadyProcessing);
    assert_eq!(code("DictationStart", 2), RefusalCode::AudioRecovering);
    // R101.12: refuses with `audio_recovering` and publishes Recovering.
    let retiring = code("DictationStart", 3);
    assert_eq!(retiring, RefusalCode::AudioRecoveringCleanupPending);
    assert!(retiring.publishes_recovering());
    assert_eq!(retiring.text(), "audio_recovering");
    assert_eq!(code("DictationStart", 5), RefusalCode::BusyTranscribingFile);
    assert_eq!(code("DictationStart", 6), RefusalCode::BusyMeeting);
    assert_eq!(code("DictationStart", 7), RefusalCode::BusyBenchmarking);
    assert_eq!(code("DictationStart", 12), RefusalCode::BusyBenchmarking);
    assert_eq!(code("DictationStart", 13), RefusalCode::BusyRecordingCorpus);
    assert_eq!(code("DictationStart", 14), RefusalCode::BusyRestarting);

    assert_eq!(code("DictationPrepare", 0), RefusalCode::DictationBusy);
    assert_eq!(code("DictationPrepare", 9), RefusalCode::MicrophoneBusy);
    assert_eq!(code("DictationPrepare", 10), RefusalCode::PreviewActive);
    assert_eq!(
        code("MicStartupBenchmark", 7),
        RefusalCode::MicBenchAlreadyRunning
    );
    assert_eq!(
        code("MicStartupBenchmark", 8),
        RefusalCode::MicBenchWaitForModelPrep
    );
    assert_eq!(code("Benchmark", 12), RefusalCode::BenchAlreadyRunning);
    assert_eq!(code("Corpus", 13), RefusalCode::CorpusAlreadyActive);
    assert_eq!(
        code("FileTranscription", 5),
        RefusalCode::FileAlreadyTranscribing
    );
    assert_eq!(code("PreviewStart", 10), RefusalCode::PreviewAlreadyActive);
}

/// Where C6.3 names a refusal string that the code does not produce, the code
/// wins (README "Contradictions"): `cleanup_pending` and `busy_restarting` are
/// printed for the retiring-capture and relaunch cells of prepare / key-down /
/// relaunch, but `dictation_refusal` returns `microphone_busy` (a retiring
/// capture is `audio_active`, checked first) and `dictation_busy` (every
/// blocker, including a committed relaunch, maps to it).
#[test]
fn retiring_capture_and_relaunch_use_the_codes_the_code_returns() {
    for request in [Request::DictationPrepare, Request::KeyDownCapture] {
        assert_eq!(
            admit(request, &occupant(3)),
            refused(RefusalCode::MicrophoneBusy)
        );
        assert_eq!(
            admit(request, &occupant(14)),
            refused(RefusalCode::DictationBusy)
        );
    }
    assert_eq!(
        admit(Request::RelaunchGate, &occupant(3)),
        refused(RefusalCode::MicrophoneBusy)
    );
    // `cleanup_pending` is reached only when nothing earlier fires, i.e. when
    // `audio_active` is false (never the case for a real retiring capture).
    let only_cleanup = Occupancy {
        cleanup_pending: true,
        ..quiet()
    };
    for request in [
        Request::DictationPrepare,
        Request::KeyDownCapture,
        Request::RelaunchGate,
    ] {
        assert_eq!(
            admit(request, &only_cleanup),
            refused(RefusalCode::CleanupPending)
        );
    }
}

#[test]
fn key_down_preview_cell_is_k_only_with_a_handoff_candidate() {
    let mut o = occupant(10);
    assert_eq!(classify(&admit(Request::KeyDownCapture, &o)), 'K');
    o.handoff_candidate = None;
    assert_eq!(classify(&admit(Request::KeyDownCapture, &o)), 'R');
}

#[test]
fn draining_pipeline_never_blocks_any_request() {
    let draining = Occupancy {
        dictation_pipeline_draining: true,
        dictation_pipeline_running: true,
        ..quiet()
    };
    for request in Request::ALL {
        let o = if request == Request::ModelPreparation {
            Occupancy {
                status: Status::Recording,
                model_matches: true,
                ..draining.clone()
            }
        } else {
            draining.clone()
        };
        assert!(
            matches!(admit(request, &o), Admission::Admit(_)),
            "{request:?} must allow a draining pipeline"
        );
    }
}

#[test]
fn dictation_start_never_reads_microphone_exclusion_facts() {
    // Supervisor-enforced: standby, unconfirmed key-down and audio_active are
    // not read (matrix cells `S`).
    let o = Occupancy {
        standby: true,
        unconfirmed_key_down: true,
        audio_active: true,
        keyboard_processing: true,
        ..quiet()
    };
    let result = admit_traced(Request::DictationStart, &o);
    assert!(matches!(result.admission, Admission::Admit(_)));
    assert_eq!(result.evaluated, check_order(Request::DictationStart));
}

#[test]
fn start_refusals_map_to_their_reply_literals() {
    let cases = [
        (RefusalCode::AppDisabled, "app_disabled", "idle"),
        (RefusalCode::BusyRestarting, "busy_restarting", "idle"),
        (RefusalCode::BusyMeeting, "busy_meeting", "idle"),
        (
            RefusalCode::BusyTranscribingFile,
            "busy_transcribing_file",
            "idle",
        ),
        (
            RefusalCode::BusyRecordingCorpus,
            "busy_recording_corpus",
            "idle",
        ),
        (RefusalCode::BusyBenchmarking, "busy_benchmarking", "idle"),
        (RefusalCode::AlreadyStarting, "already_starting", "starting"),
        (
            RefusalCode::AlreadyRecording,
            "already_recording",
            "recording",
        ),
        (
            RefusalCode::AudioRecovering,
            "audio_recovering",
            "recovering",
        ),
        (
            RefusalCode::AlreadyProcessing,
            "already_processing",
            "processing",
        ),
        (
            RefusalCode::AudioRecoveringCleanupPending,
            "audio_recovering",
            "recovering",
        ),
    ];
    for (code, ty, state) in cases {
        let (reply_type, status) = code.reply().expect("typed reply");
        assert_eq!(reply_type.as_str(), ty);
        assert_eq!(status.as_str(), state);
        assert_eq!(code.text(), ty);
    }
    assert_eq!(RefusalCode::DictationBusy.reply(), None);
}
