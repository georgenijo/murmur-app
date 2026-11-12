use super::*;

const A: Bounds = (8_000, 72_000); // 4 s of speech
const B: Bounds = (88_000, 152_000); // 4 s
const C: Bounds = (168_000, 184_000); // 1 s

fn raw() -> Vec<f32> {
    (0..16_000 * 14).map(|i| (i as f32 * 0.37).sin()).collect()
}

fn concat(raw: &[f32], bounds: &[Bounds]) -> Vec<f32> {
    bounds
        .iter()
        .flat_map(|&(start, end)| raw[start..end].iter().copied())
        .collect()
}

fn snapshot(segments: &[Bounds], scored_samples: usize) -> StreamSnapshot {
    StreamSnapshot {
        segments: segments.to_vec(),
        scored_samples,
    }
}

fn commit_plan(reuse: &mut StreamReuse, snap: &StreamSnapshot) -> CommitPlan {
    match reuse.plan(1, snap) {
        Plan::Commit(plan) => plan,
        Plan::Idle => panic!("idle"),
        Plan::Off => panic!("off"),
    }
}

/// Two committed pieces (A, then B with A as its overlap).
fn two_pieces(raw: &[f32]) -> StreamReuse {
    let mut reuse = StreamReuse::default();
    reuse.begin(1);
    let plan = commit_plan(&mut reuse, &snapshot(&[A], A.1 + EDGE_MARGIN));
    let piece = plan.piece(&raw[plan.raw_start..]).unwrap();
    assert_eq!(
        reuse.commit(1, plan, piece, "First complete sentence."),
        Committed::Stored
    );
    let plan = commit_plan(&mut reuse, &snapshot(&[A, B], B.1 + EDGE_MARGIN));
    assert_eq!(plan.overlap.len(), 64_000, "the last piece is the overlap");
    let piece = plan.piece(&raw[plan.raw_start..]).unwrap();
    assert_eq!(
        reuse.commit(
            1,
            plan,
            piece,
            "First complete sentence. Second complete sentence."
        ),
        Committed::Stored
    );
    reuse
}

#[test]
fn pieces_are_vad_trimmed_and_the_tail_is_stitched_after_the_overlap() {
    let raw = raw();
    let mut reuse = two_pieces(&raw);
    let filtered = concat(&raw, &[A, B, C]);
    let plan = reuse.take_final(1, Some(&[A, B, C]), &filtered).unwrap();
    // The tail starts at the last committed piece, in trimmed coordinates.
    assert_eq!(plan.start, 64_000);
    assert_eq!(filtered.len() - plan.start, 64_000 + 16_000);
    assert_eq!(plan.pieces, 2);
    assert_eq!(
        plan.stitch("Second complete sentence. Third bit."),
        Some("First complete sentence. Second complete sentence. Third bit.".into())
    );
    // Exact punctuation, capitalisation and word boundary, or whole clip.
    assert!(plan
        .stitch("Second complete sentence, Third bit.")
        .is_none());
    assert!(plan
        .stitch("second complete sentence. Third bit.")
        .is_none());
    assert!(plan.stitch("Second complete sentence.extra").is_none());
    assert!(plan.stitch("Second complete sentence.").is_none());
    assert!(plan.stitch("").is_none());
    // The plan is consumed at most once.
    assert_eq!(
        reuse
            .take_final(1, Some(&[A, B, C]), &filtered)
            .unwrap_err(),
        Reason::Superseded
    );
}

#[test]
fn committed_pieces_that_cover_all_kept_audio_need_no_tail_decode() {
    let raw = raw();
    let mut reuse = two_pieces(&raw);
    let filtered = concat(&raw, &[A, B]);
    let plan = reuse.take_final(1, Some(&[A, B]), &filtered).unwrap();
    assert_eq!(
        plan.complete_text().as_deref(),
        Some("First complete sentence. Second complete sentence.")
    );
    assert!(plan.stitch("Second complete sentence. More.").is_none());
}

#[test]
fn a_cut_needs_new_speech_a_real_pause_and_a_final_segment() {
    let raw = raw();
    let _ = &raw;
    let mut reuse = StreamReuse::default();
    reuse.begin(1);
    // Too little speech.
    let short = (0, 40_000);
    assert!(matches!(
        reuse.plan(1, &snapshot(&[short], 40_000 + EDGE_MARGIN)),
        Plan::Idle
    ));
    // Enough speech, but the last segment may still be growing.
    assert!(matches!(
        reuse.plan(1, &snapshot(&[A], A.1 + EDGE_MARGIN - 1)),
        Plan::Idle
    ));
    // A later segment makes the earlier one final, if the pause is real.
    let touching = (A.1 + MIN_PAUSE - 1, A.1 + MIN_PAUSE + 20_000);
    assert!(matches!(
        reuse.plan(1, &snapshot(&[A, touching], touching.1 + 100)),
        Plan::Idle
    ));
    assert!(matches!(
        reuse.plan(1, &snapshot(&[A, B], B.0 + 100)),
        Plan::Commit(_)
    ));
}

#[test]
fn a_decode_whose_overlap_reads_differently_ends_reuse() {
    let raw = raw();
    let mut reuse = StreamReuse::default();
    reuse.begin(1);
    let plan = commit_plan(&mut reuse, &snapshot(&[A], A.1 + EDGE_MARGIN));
    let piece = plan.piece(&raw[plan.raw_start..]).unwrap();
    reuse.commit(1, plan, piece, "First complete sentence.");
    let plan = commit_plan(&mut reuse, &snapshot(&[A, B], B.1 + EDGE_MARGIN));
    let piece = plan.piece(&raw[plan.raw_start..]).unwrap();
    assert_eq!(
        reuse.commit(
            1,
            plan,
            piece,
            "First complete sentence, Second complete sentence."
        ),
        Committed::Off
    );
    let filtered = concat(&raw, &[A, B, C]);
    assert_eq!(
        reuse
            .take_final(1, Some(&[A, B, C]), &filtered)
            .unwrap_err(),
        Reason::OverlapMismatch
    );
}

#[test]
fn a_piece_with_too_few_new_words_waits_for_a_later_cut() {
    let raw = raw();
    let mut reuse = StreamReuse::default();
    reuse.begin(1);
    let plan = commit_plan(&mut reuse, &snapshot(&[A], A.1 + EDGE_MARGIN));
    let piece = plan.piece(&raw[plan.raw_start..]).unwrap();
    assert_eq!(reuse.commit(1, plan, piece, "Hi."), Committed::Deferred);
    // The same cut is not tried again; the next segment is.
    assert!(matches!(
        reuse.plan(1, &snapshot(&[A], A.1 + EDGE_MARGIN + 5_000)),
        Plan::Idle
    ));
    let plan = commit_plan(&mut reuse, &snapshot(&[A, B], B.1 + EDGE_MARGIN));
    assert_eq!(plan.segments, vec![A, B], "deferred audio is kept");
    assert!(plan.overlap.is_empty());
}

#[test]
fn a_stale_cancelled_or_other_recording_never_reads_or_changes_state() {
    let raw = raw();
    let mut reuse = StreamReuse::default();
    reuse.begin(8);
    reuse.begin(7); // A late ticker cannot replace a newer session.
    assert!(matches!(
        reuse.plan(7, &snapshot(&[A], A.1 + EDGE_MARGIN)),
        Plan::Off
    ));
    let plan = match reuse.plan(8, &snapshot(&[A], A.1 + EDGE_MARGIN)) {
        Plan::Commit(plan) => plan,
        _ => panic!("expected a piece"),
    };
    let piece = plan.piece(&raw[plan.raw_start..]).unwrap();
    // A worker of another recording cannot commit this one's piece.
    let other = CommitPlan {
        id: 7,
        ..plan.clone()
    };
    assert_eq!(
        reuse.commit(7, other, piece.clone(), "Old worker text here."),
        Committed::Off
    );
    assert_eq!(
        reuse.take_final(7, Some(&[A]), &[]).unwrap_err(),
        Reason::Inactive
    );
    reuse.commit(8, plan, piece, "Current worker text here.");
    reuse.fail(7, Reason::DecodeError); // does not touch recording 8
    assert!(reuse.is_active(8));
    reuse.clear(8); // cancel / end of recording
    assert!(!reuse.is_active(8));
    assert_eq!(
        reuse
            .take_final(8, Some(&[A]), &concat(&raw, &[A]))
            .unwrap_err(),
        Reason::Superseded
    );
}

#[test]
fn changed_audio_changed_segments_or_a_batch_vad_cannot_be_stitched() {
    let raw = raw();
    let filtered = concat(&raw, &[A, B, C]);

    // One changed sample in the last committed piece.
    let mut changed = filtered.clone();
    changed[64_000 + 5] += 0.001;
    assert_eq!(
        two_pieces(&raw)
            .take_final(1, Some(&[A, B, C]), &changed)
            .unwrap_err(),
        Reason::AudioMismatch
    );
    // The final VAD's segments are not the ones decoded during recording.
    let shifted = (A.0, A.1 + 160);
    assert_eq!(
        two_pieces(&raw)
            .take_final(1, Some(&[shifted, B, C]), &filtered)
            .unwrap_err(),
        Reason::BoundsMismatch
    );
    // Kept audio is not exactly the final segments (coverage).
    assert_eq!(
        two_pieces(&raw)
            .take_final(1, Some(&[A, B, C]), &filtered[..filtered.len() - 1])
            .unwrap_err(),
        Reason::BoundsMismatch
    );
    // The batch VAD produced the audio: nothing can be compared.
    assert_eq!(
        two_pieces(&raw).take_final(1, None, &filtered).unwrap_err(),
        Reason::VadNotIncremental
    );
    // Final VAD dropped a committed segment.
    assert_eq!(
        two_pieces(&raw)
            .take_final(1, Some(&[A]), &concat(&raw, &[A]))
            .unwrap_err(),
        Reason::BoundsMismatch
    );
}

#[test]
fn speech_that_never_pauses_within_one_window_ends_reuse() {
    let mut reuse = StreamReuse::default();
    reuse.begin(1);
    // 21 s of speech with no pause long enough to cut.
    let seg = (0, 21 * SAMPLE_RATE);
    assert!(matches!(
        reuse.plan(1, &snapshot(&[seg], seg.1 + 10)),
        Plan::Off
    ));
    assert_eq!(reuse.failed, Some(Reason::WindowCap));
}

#[test]
fn a_tail_longer_than_one_window_or_a_long_recording_falls_back() {
    let raw = raw();
    let mut reuse = two_pieces(&raw);
    // Pretend the last piece starts early so the tail exceeds one window.
    reuse.last_start = 0;
    let long_tail = vec![0.0_f32; MAX_WINDOW + 1];
    reuse.last_piece = vec![0.0; 128_000];
    reuse.trimmed_len = 128_000;
    reuse.last_hash = hash_samples(&long_tail[..128_000]);
    let mut bounds = vec![A, B];
    bounds.push((200_000, 200_000 + MAX_WINDOW));
    let mut filtered = long_tail[..128_000].to_vec();
    filtered.extend(vec![0.0; MAX_WINDOW]);
    assert_eq!(
        reuse.take_final(1, Some(&bounds), &filtered).unwrap_err(),
        Reason::TailTooLong
    );

    let mut reuse = two_pieces(&raw);
    let snap = snapshot(&[A], MAX_RECORDING + 1);
    assert!(matches!(reuse.plan(1, &snap), Plan::Off));
    assert_eq!(reuse.failed, Some(Reason::LongRecording));
}

#[test]
fn nothing_committed_means_no_reuse() {
    let mut reuse = StreamReuse::default();
    reuse.begin(1);
    assert_eq!(
        reuse.take_final(1, Some(&[A]), &[0.0; 10]).unwrap_err(),
        Reason::NoCommit
    );
}

#[test]
fn committed_segments_changing_under_the_stream_end_reuse() {
    let raw = raw();
    let mut reuse = two_pieces(&raw);
    // The VAD now reports different bounds for an already committed segment.
    let shifted = (A.0, A.1 + 160);
    assert!(matches!(
        reuse.plan(1, &snapshot(&[shifted, B, C], C.1 + EDGE_MARGIN)),
        Plan::Off
    ));
    assert_eq!(reuse.failed, Some(Reason::BoundsMismatch));
}

#[test]
fn memory_does_not_grow_with_the_recording() {
    // Only the last piece (at most one window) is retained, never the
    // recording or the committed prefix.
    let raw = raw();
    let reuse = two_pieces(&raw);
    assert_eq!(reuse.last_piece.len(), 64_000);
    assert!(reuse.last_piece.len() <= MAX_WINDOW);
}
