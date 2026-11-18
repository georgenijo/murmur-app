//! A preserved defect is credited only after its defining behaviour was witnessed
//! (D-S2c-1 d; Sol 6.2 major 3). The old rule credited every PD of a row at CP1, so
//! `PD-16` counted although the later terminal never ran and `PD-18` counted on an
//! ordinary short idle.

use murmur_dictation_core::decision::OpStatus;
use murmur_dictation_core::effects::Effect;
use murmur_dictation_core::status::Status;

use murmur_dictation_core::ids::RecordingId;

use super::harness::*;
use crate::dictation_shadow::pd::{credited, PdEvidence, PD_DEFS};
use crate::dictation_shadow::types::{EffectResult, ShadowMsg};

fn credited_count(h: &Harness, pd: u8) -> u64 {
    h.stats().pd_witness.get(&pd).copied().unwrap_or(0)
}

#[test]
fn a_clean_short_idle_credits_pd_1_and_pd_15_and_not_pd_18() {
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    assert_clean(&h);
    assert_eq!(credited_count(&h, 1), 1);
    assert_eq!(credited_count(&h, 15), 1);
    assert_eq!(
        credited_count(&h, 18),
        0,
        "PD-18 is a start in the teardown tail, not an ordinary short idle: {:?}",
        h.stats().pd_witness
    );
    let row = h.stats().per_row[&(216_u16, 1)];
    assert_eq!(row.pd_witness, 2);
    assert_eq!(row.compared, 1);
}

#[test]
fn nothing_is_credited_before_the_record_closes() {
    // The header alone has not witnessed the later effects.
    let mut h = harness();
    h.feed_header(&cancel(1, 1));
    assert!(h.stats().pd_witness.is_empty());
    h.feed_later(&cancel(1, 1));
    assert_eq!(credited_count(&h, 16), 1);
}

#[test]
fn pd_16_is_not_credited_when_the_later_terminal_is_missing() {
    let mut h = harness();
    let mut flight = cancel(1, 1);
    flight.later.retain(
        |m| !matches!(m, ShadowMsg::Witness { effect, .. } if matches!(effect.effect, Effect::Terminal(_))),
    );
    h.feed(&flight);
    assert_eq!(h.stats().mismatched_root, 1);
    assert!(
        h.stats().pd_witness.is_empty(),
        "{:?}",
        h.stats().pd_witness
    );
}

#[test]
fn a_pd_row_with_a_different_effect_list_is_a_mismatch_and_not_credited() {
    let mut h = harness();
    let mut flight = short_idle(1, 1);
    flight.header.pre_effects.remove(3);
    h.feed(&flight);
    assert_eq!(h.stats().mismatched_root, 1);
    assert!(h.stats().pd_witness.is_empty());
}

#[test]
fn pd_18_needs_a_clean_teardown_row_before_the_start() {
    // The start alone (R101.12 with nothing before it) is not the defect.
    let mut h = harness();
    h.feed(&start_during_cleanup(1, 1));
    assert_clean(&h);
    assert_eq!(credited_count(&h, 18), 0);

    // After the attempt's clean teardown (R216.1, same recording) it is.
    let mut h = harness();
    h.feed(&short_idle(1, 1));
    h.feed(&start_during_cleanup(2, 2));
    assert_clean(&h);
    assert_eq!(credited_count(&h, 18), 1);

    // A teardown that mismatched is not a clean window.
    let mut h = harness();
    let mut bad = short_idle(1, 1);
    bad.header.live_post_d.status = Status::Recording;
    h.feed(&bad);
    h.feed(&start_during_cleanup(2, 2));
    assert_eq!(credited_count(&h, 18), 0);

    // Another recording's teardown does not open it.
    let mut h = harness();
    h.feed(&idle_for(1, 1, 41));
    h.feed(&start_during_cleanup(2, 2));
    assert_eq!(credited_count(&h, 18), 0);
}

#[test]
fn pd_4_needs_the_rendezvous_a_second_ticker_and_the_later_success() {
    // R104.2 completes the rendezvous (and registers the ticker), then the late
    // `Ready` for the same recording wins a stale terminal and re-registers the
    // ticker: the duplicate. PD-4 is credited only once the recording's later
    // successful performance outcome (`PGF(Completed)`) is witnessed (D-S2c-2 g).
    let duplicate = Live {
        ticker_before: RID,
        ..Live::default()
    };
    let mut h = harness();
    h.feed(&ready_publish(1, 1, OpStatus::Outstanding));
    h.feed(&ready_while_recording(2, 2, duplicate));
    assert_clean(&h);
    assert_eq!(
        credited_count(&h, 4),
        0,
        "the outcome has not been witnessed yet: {:?}",
        h.stats().pd_witness
    );
    h.feed(&pipeline_success(3, 3));
    assert_clean(&h);
    assert_eq!(credited_count(&h, 4), 1, "{:?}", h.stats().pd_witness);

    // Without the rendezvous before it, it is not.
    let mut h = harness();
    h.feed(&ready_while_recording(1, 1, duplicate));
    h.feed(&pipeline_success(2, 2));
    assert_clean(&h);
    assert_eq!(credited_count(&h, 4), 0);

    // Without a second ticker (the gate suppressed it) the defect did not happen.
    let mut h = harness();
    h.feed(&ready_publish(1, 1, OpStatus::Outstanding));
    h.feed(&ready_while_recording(2, 2, Live::default()));
    h.feed(&pipeline_success(3, 3));
    assert_clean(&h);
    assert_eq!(credited_count(&h, 4), 0);

    // Another recording's success does not settle it: the stop tail of recording 41
    // completes 41's performance, although RID is the one that is current.
    let mut h = harness();
    h.feed(&ready_publish(1, 1, OpStatus::Outstanding));
    h.feed(&ready_while_recording(2, 2, duplicate));
    h.feed(&pipeline_success_for(3, 3, RecordingId(41)));
    assert_eq!(
        credited_count(&h, 4),
        0,
        "borrowed another recording's success: {:?}",
        h.stats().pd_witness
    );
    // Its own recording's success still does.
    h.feed(&pipeline_success(4, 4));
    assert_eq!(credited_count(&h, 4), 1, "{:?}", h.stats().pd_witness);

    // A losing terminal claim is not the defect: the stale terminal did not win.
    let mut h = harness();
    h.feed(&ready_publish(1, 1, OpStatus::Outstanding));
    h.feed(&ready_while_recording(
        2,
        2,
        Live {
            ticker_before: RID,
            term_won: false,
            ..Live::default()
        },
    ));
    h.feed(&pipeline_success(3, 3));
    assert_clean(&h);
    assert_eq!(credited_count(&h, 4), 0, "a losing claim credits nothing");
}

#[test]
fn pd_10_needs_the_terminal_claim_to_have_lost() {
    let mut h = harness();
    h.feed(&capture_failed(1, 1, Live::default()));
    assert_clean(&h);
    assert_eq!(
        credited_count(&h, 10),
        0,
        "the claim won: nothing was dropped"
    );

    let mut h = harness();
    h.feed(&capture_failed(
        1,
        1,
        Live {
            term_won: false,
            ..Live::default()
        },
    ));
    assert_clean(&h);
    assert_eq!(credited_count(&h, 10), 1);
}

#[test]
fn pd_9_is_an_abandonment_after_the_perf_register() {
    // The start future dropped after `PR` and before the capture started.
    let flight = start(1, 1, Live::default());
    let pr = witness_at(&flight, |e| matches!(e, Effect::PerfRegister));
    let mut h = harness();
    h.feed_header(&flight);
    for msg in flight.later.iter().take(pr + 1).cloned() {
        h.send(msg);
    }
    h.send(ShadowMsg::Close {
        rec: flight.header.rec,
        completed: false,
    });
    assert_clean(&h);
    assert_eq!(credited_count(&h, 9), 1);

    // Dropped before `PR` ran: no orphan.
    let mut h = harness();
    h.feed_header(&flight);
    h.send(ShadowMsg::Close {
        rec: flight.header.rec,
        completed: false,
    });
    assert_eq!(credited_count(&h, 9), 0);

    // A completed start (the capture did start) is not the defect.
    let mut h = harness();
    h.feed(&start(1, 1, Live::default()));
    assert_eq!(credited_count(&h, 9), 0);
}

#[test]
fn every_pd_definition_names_a_c3_row_and_real_effect_codes() {
    let codes = [
        "ST", "ID+", "EM", "EV", "PRES", "TERM", "PA", "PR", "PB", "PC", "CX-", "CX+", "KB", "MC",
        "TL", "AX", "GA", "PCC", "PH", "TI", "TS", "SD", "PM", "CS", "CP", "CC", "RS", "PS", "REL",
        "TRQ", "OBS", "ACC", "PCF", "SPEC", "PE", "PGF", "SDE",
    ];
    for def in PD_DEFS {
        assert!(
            murmur_dictation_core::rows::site_of_row(def.row).is_some(),
            "PD-{} names {}, which is not a C3 row",
            def.pd,
            def.row
        );
        for code in def.need.iter().chain(def.absent) {
            assert!(codes.contains(code), "PD-{} names effect `{code}`", def.pd);
        }
        for prior in def.prior {
            assert!(
                murmur_dictation_core::rows::site_of_row(prior).is_some(),
                "PD-{} names prior row {prior}",
                def.pd
            );
        }
    }
    // The PD numbers are the C7.1 ones; PD-11, PD-14 and PD-17 are not row
    // signatures.
    let numbers: Vec<u8> = PD_DEFS.iter().map(|d| d.pd).collect();
    for absent in [11, 14, 17] {
        assert!(!numbers.contains(&absent));
    }
}

#[test]
fn credit_needs_defining_effects_results_and_windows() {
    let ran = [("TERM", EffectResult::Claimed { won: false })];
    let evidence = PdEvidence {
        row: "R102.2",
        ran: &ran,
        tickers: &[],
        recording: RID,
        completed: true,
        prior_rows: &[],
    };
    // R102.2's PD-3 needs the idle emit that never ran here; PD-10 needs the lost claim.
    assert_eq!(credited(&evidence), vec![10]);
    let won = [("TERM", EffectResult::Claimed { won: true })];
    assert!(credited(&PdEvidence {
        ran: &won,
        ..evidence
    })
    .is_empty());
}

#[test]
fn an_effect_the_definition_says_must_be_absent_denies_the_credit() {
    // PD-13 (R204a.6) is a row that does nothing: the credit needs `TERM` and `EM` to
    // be absent from what ran. PD-9 needs `CS` (the capture start) absent.
    let none: [(&'static str, EffectResult); 0] = [];
    let base = PdEvidence {
        row: "R204a.6",
        ran: &none,
        tickers: &[],
        recording: RID,
        completed: true,
        prior_rows: &[],
    };
    assert_eq!(credited(&base), vec![13]);
    for code in ["TERM", "EM"] {
        let ran = [(code, EffectResult::Ran)];
        assert!(
            credited(&PdEvidence { ran: &ran, ..base }).is_empty(),
            "{code} ran, so PD-13 did not happen"
        );
    }
    let pr = [("PR", EffectResult::Ran)];
    let pd9 = PdEvidence {
        row: "R101.15",
        ran: &pr,
        completed: false,
        ..base
    };
    assert_eq!(credited(&pd9), vec![9]);
    let started = [("PR", EffectResult::Ran), ("CS", EffectResult::Ran)];
    assert!(credited(&PdEvidence {
        ran: &started,
        ..pd9
    })
    .is_empty());
}

#[test]
fn abandonment_decides_which_flights_a_definition_applies_to() {
    // PD-9 is an abandonment: a completed flight never credits it. Every other PD is
    // about a flight that ran to its close: an abandoned flight never credits one.
    let pr = [("PR", EffectResult::Ran)];
    let pd9 = PdEvidence {
        row: "R101.15",
        ran: &pr,
        tickers: &[],
        recording: RID,
        completed: true,
        prior_rows: &[],
    };
    assert!(credited(&pd9).is_empty(), "a completed start is not PD-9");
    assert_eq!(
        credited(&PdEvidence {
            completed: false,
            ..pd9
        }),
        vec![9]
    );

    let idle = [
        ("CX-", EffectResult::Ran),
        ("ST", EffectResult::Ran),
        ("KB", EffectResult::Ran),
        ("EM", EffectResult::Emitted { ok: true }),
    ];
    let pd1 = PdEvidence {
        row: "R216.1",
        ran: &idle,
        tickers: &[],
        recording: RID,
        completed: true,
        prior_rows: &[],
    };
    assert_eq!(credited(&pd1), vec![1]);
    assert!(credited(&PdEvidence {
        completed: false,
        ..pd1
    })
    .is_empty());
}
