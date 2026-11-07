//! Existing pane workflows; service and editor ownership stays injected.
use super::*;
impl Settings {
    pub(super) fn extra(&mut self, w: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut out = vec![];
        let r = self.model.runtime();
        let settings = self.model.settings().clone();
        let advanced = self.model.advanced();
        macro_rules! action {
            ($id:expr,$label:expr,$action:expr,$danger:expr,$disabled:expr) => {
                out.push(
                    self.action_button($id, $label, $action, $danger, $disabled, cx)
                        .into_any_element(),
                )
            };
        }
        macro_rules! editor {
            ($id:expr,$label:expr,$destination:expr) => {
                out.push(
                    self.editor_button($id, $label, $destination, cx)
                        .into_any_element(),
                )
            };
        }
        macro_rules! text {
            ($value:expr) => {
                out.push(
                    div()
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .child($value)
                        .into_any_element(),
                )
            };
        }
        match self.model.page {
            Page::General => {
                out.push(
                    self.route_button("shortcuts", "Keyboard shortcuts", Page::Shortcuts, cx)
                        .into_any_element(),
                );
                editor!("setup", "Run setup assistant…", Destination::Setup);
                editor!(
                    "checklist",
                    "Show feature checklist",
                    Destination::Checklist
                );
                if advanced {
                    if let Some(requested) = self.calibration {
                        let offset = r.calibration.map(|c| c.applied).unwrap_or(requested);
                        let (min, max) = r.calibration.map(|c| (c.min, c.max)).unwrap_or((-12, 12));
                        text!(format!("Overlay preview: {offset:+} pt"));
                        for (id, label, next) in [
                            ("cal-up", "Move overlay up one point", offset - 1),
                            ("cal-down", "Move overlay down one point", offset + 1),
                            ("cal-default", "Preview default", 0),
                        ] {
                            let e = cx.entity();
                            out.push(
                                Button::new(id, label)
                                    .with_scroll_parent(Some(&self.scroll))
                                    .track_focus(&self.focus_for(id, cx))
                                    .disabled(self.model.pending || !(min..=max).contains(&next))
                                    .on_click(move |_, _, cx| {
                                        e.update(cx, |s, cx| {
                                            s.action(
                                                Action::Calibration {
                                                    offset: next,
                                                    finish: false,
                                                },
                                                false,
                                                cx,
                                            );
                                        })
                                    })
                                    .into_any_element(),
                            );
                        }
                        action!(
                            "cal-save",
                            "Save position",
                            Action::Calibration {
                                offset: requested,
                                finish: true
                            },
                            false,
                            false
                        );
                        action!(
                            "cal-cancel",
                            "Cancel calibration",
                            Action::CancelCalibration,
                            false,
                            false
                        );
                    } else {
                        let e = cx.entity();
                        out.push(
                            Button::new("calibrate", "Calibrate overlay position…")
                                .with_scroll_parent(Some(&self.scroll))
                                .track_focus(&self.focus_for("calibrate", cx))
                                .disabled(self.model.pending)
                                .on_click(move |_, _, cx| {
                                    e.update(cx, |s, cx| {
                                        let offset = s.model.settings()["overlayVerticalOffset"]
                                            .as_i64()
                                            .unwrap_or(0);
                                        s.calibration = Some(offset);
                                        // The opener is unmounted. Cancel stays enabled even
                                        // while a preview reply is pending.
                                        s.restore_focus = Some("cal-cancel");
                                        s.action(
                                            Action::Calibration {
                                                offset,
                                                finish: false,
                                            },
                                            false,
                                            cx,
                                        );
                                    })
                                })
                                .into_any_element(),
                        );
                        action!(
                            "reset-overlay",
                            "Reset overlay position",
                            Action::Calibration {
                                offset: 0,
                                finish: true
                            },
                            false,
                            settings["overlayVerticalOffset"] == 0
                        );
                    }
                    action!(
                        "reset-statistics",
                        "Reset statistics…",
                        Action::ResetStatistics,
                        true,
                        false
                    );
                }
                if !r.internal_build {
                    text!(r.update.label());
                    if let Some(percent) = r.update_percent
                        && r.update == Update::Downloading
                    {
                        text!(format!("Downloaded · {percent}%"));
                    }
                    let (label, action, busy) = match r.update {
                        Update::Available => ("Download update", Action::DownloadUpdate, false),
                        Update::Ready => ("Restart to update…", Action::RestartUpdate, false),
                        Update::Checking | Update::Downloading => {
                            ("Checking for updates…", Action::CheckUpdate, true)
                        }
                        _ => ("Check for updates", Action::CheckUpdate, false),
                    };
                    action!("updates", label, action, false, busy);
                    editor!(
                        "update-details",
                        "Update details…",
                        Destination::UpdateDetails
                    );
                }
                if !r.version.is_empty() {
                    text!(format!("Murmur v{}", r.version));
                }
            }
            Page::Shortcuts => {
                #[cfg(any(test, feature = "gallery"))]
                out.extend(self.fixture_capture_buttons(cx));
                let [recording, paste] = shortcut_reference(&settings);
                text!(recording);
                out.push(
                    self.route_button(
                        "shortcut-recording",
                        "Change recording shortcut",
                        Page::Recording,
                        cx,
                    )
                    .into_any_element(),
                );
                text!(paste);
                out.push(
                    self.route_button(
                        "shortcut-delivery",
                        "Change Paste last shortcut",
                        Page::Delivery,
                        cx,
                    )
                    .into_any_element(),
                );
                for label in [
                    "Command palette · ⌘K",
                    "Search transcripts · ⌘F",
                    "Open settings… · ⌘,",
                    "Open Performance Lab · ⌘L",
                ] {
                    text!(label);
                }
            }
            Page::Customize => {
                out.push(
                    self.route_button("customize-text", "Text & vocabulary…", Page::Text, cx)
                        .into_any_element(),
                );
                editor!(
                    "customize-commands",
                    "Voice commands…",
                    Destination::VoiceCommands
                );
                editor!(
                    "customize-overrides",
                    "App overrides…",
                    Destination::AppOverrides
                );
                out.push(
                    self.route_button("customize-modes", "Modes…", Page::Modes, cx)
                        .into_any_element(),
                );
            }
            Page::Recording => {
                text!(if r.inventory_stale {
                    "Microphone list is stale. A saved device is not proven available."
                } else if r.recording || r.monitor_paused {
                    "Microphone test paused during capture"
                } else {
                    "Local microphone test · no audio retained"
                });
                if let Some((rms, peak)) = r.meter {
                    text!(format!(
                        "Input level: {:.0}% · Peak: {:.0}% · {}",
                        rms * 100.,
                        peak * 100.,
                        if peak >= 0.98 {
                            "Clipping — lower input gain"
                        } else if rms < 0.02 {
                            "Quiet — speak closer to the microphone"
                        } else {
                            "Signal detected"
                        }
                    ));
                }
                if r.accessibility == Some(false) {
                    text!("Accessibility permission is required for keyboard detection.");
                    action!(
                        "accessibility",
                        "Grant Accessibility…",
                        Action::Accessibility,
                        false,
                        false
                    );
                }
                if advanced {
                    text!(format!("Smart Auto-Stop model · {}", r.smart_turn.label()));
                    if r.smart_turn == Install::Installed {
                        action!(
                            "remove-smart-turn",
                            "Remove Smart Auto-Stop model…",
                            Action::RemoveSmartTurn,
                            true,
                            r.recording
                        );
                    } else {
                        action!(
                            "install-smart-turn",
                            "Download Smart Auto-Stop model",
                            Action::InstallSmartTurn,
                            false,
                            r.recording || r.smart_turn == Install::Busy
                        );
                    }
                    if settings["soundCuesEnabled"] == true {
                        for (id, label, cue) in [
                            ("cue-start", "Preview start cue", Cue::Start),
                            ("cue-stop", "Preview stop cue", Cue::Stop),
                            ("cue-success", "Preview success cue", Cue::Success),
                            ("cue-failure", "Preview failure cue", Cue::Failure),
                        ] {
                            action!(id, label, Action::PreviewCue(cue), false, false);
                        }
                    }
                    if settings["smartAutoMicrophoneEnabled"] == true {
                        for device in &r.devices {
                            let id = device.id.clone();
                            let approved = settings["smartAutoApprovedDeviceIds"]
                                .as_array()
                                .is_some_and(|a| a.contains(&json!(id)));
                            let label = format!("Approve {} for Smart Auto", device.label);
                            let e = cx.entity();
                            out.push(div().flex().items_center().justify_between().gap(px(12.)).child(label.clone()).child(Toggle::new(SharedString::from(format!("approve-{}",id)),label,approved).with_scroll_parent(&self.scroll).disabled(self.model.pending||r.recording||r.inventory_stale).on_change(move|v,_,cx|e.update(cx,|s,cx|{let mut ids=s.model.settings()["smartAutoApprovedDeviceIds"].as_array().cloned().unwrap_or_default();ids.retain(|x|x!=&json!(id));if *v {ids.push(json!(id));}s.request(RequestKind::Patch{key:"smartAutoApprovedDeviceIds",value:json!(ids)},cx);}))).into_any_element());
                            if approved {
                                let id = device.id.clone();
                                let preferred = settings["smartAutoPreferredDeviceIds"]
                                    .as_array()
                                    .is_some_and(|a| a.first() == Some(&json!(id)));
                                let label = format!("Prefer {}", device.label);
                                let e = cx.entity();
                                out.push(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap(px(12.))
                                        .child(label.clone())
                                        .child(
                                            Button::new(SharedString::from(format!("prefer-{id}")), label)
                                                .with_scroll_parent(Some(&self.scroll))
                                                .caption(if preferred { "Preferred" } else { "Prefer" })
                                                .toggled(preferred)
                                                .disabled(self.model.pending || r.recording)
                                                .on_click(move |_, _, cx| e.update(cx, |s, cx| {
                                                    let settings = s.model.settings();
                                                    let mut ids = settings["smartAutoPreferredDeviceIds"]
                                                        .as_array().cloned().unwrap_or_default();
                                                    ids.retain(|x| x != &json!(id) && settings["smartAutoApprovedDeviceIds"]
                                                        .as_array().is_some_and(|a| a.contains(x)));
                                                    ids.insert(0, json!(id));
                                                    s.request(RequestKind::Patch {
                                                        key: "smartAutoPreferredDeviceIds",
                                                        value: json!(ids),
                                                    }, cx);
                                                })),
                                        )
                                        .into_any_element(),
                                );
                            }
                        }
                    }
                }
            }
            Page::Delivery => {
                if advanced {
                    editor!("app-overrides", "App overrides…", Destination::AppOverrides);
                }
            }
            Page::Meetings => {
                text!("Meetings are stored separately from dictation history.");
                if settings["meetingSuggestionsEnabled"] == true {
                    action!(
                        "calendar",
                        "Open Calendar settings…",
                        Action::CalendarSettings,
                        false,
                        false
                    );
                }
                if r.speakers_supported != Some(false) {
                    text!(format!("Remote speaker model · {}", r.speakers.label()));
                    if r.speakers == Install::Installed {
                        action!(
                            "remove-speakers",
                            "Remove remote speaker model…",
                            Action::RemoveSpeakers,
                            true,
                            r.recording
                        );
                    } else {
                        action!(
                            "install-speakers",
                            "Download remote speaker model",
                            Action::InstallSpeakers,
                            false,
                            r.recording || r.speakers == Install::Busy
                        );
                    }
                }
            }
            Page::Text => {
                editor!("vocabulary", "Vocabulary…", Destination::Vocabulary);
                editor!("aliases", "Aliases…", Destination::Aliases);
                editor!("knowledge", "Knowledge…", Destination::Knowledge);
                if advanced {
                    editor!("commands", "Voice commands…", Destination::VoiceCommands);
                    if settings["codeVocabEnabled"] == true {
                        editor!(
                            "project-scan",
                            "Manage project scan…",
                            Destination::ProjectScan
                        );
                    }
                }
            }
            Page::Ai => {
                out.push(
                    self.route_button("speech-to-text", "Speech-to-text…", Page::Transcription, cx)
                        .into_any_element(),
                );
            }
            Page::Transcription => {
                text!(format!("Transcription model · {}", r.model_install.label()));
                editor!(
                    "model-storage",
                    "Manage model storage…",
                    Destination::ModelStorage
                );
            }
            Page::Modes => {
                editor!("modes", "Manage Modes…", Destination::Modes);
            }
            Page::Appearance => {
                editor!(
                    "appearance",
                    "Manage appearance and themes…",
                    Destination::Appearance
                );
            }
            Page::Diagnostics => {
                editor!("diagnostics", "Open Diagnostics…", Destination::Diagnostics);
            }
            Page::Performance => {
                text!(
                    "Choose installed models, then explicitly run or cancel. Dictation model selection stays unchanged."
                );
                for model in [
                    "parakeet-tdt-0.6b-v3-coreml",
                    "tiny.en",
                    "base.en",
                    "small.en",
                    "medium.en",
                    "large-v3-turbo",
                    "parakeet-tdt-0.6b-v2-fp16",
                ] {
                    let selected = self.lab_selection.contains(model);
                    let e = cx.entity();
                    out.push(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .child(model)
                            .child(
                                Toggle::new(model, format!("Include {model}"), selected)
                                    .with_scroll_parent(&self.scroll)
                                    .disabled(
                                        r.lab_running
                                            || self.model.pending
                                            || !r.installed_models.as_ref().is_some_and(|models| {
                                                models.iter().any(|id| id == model)
                                            }),
                                    )
                                    .track_focus(&self.focus_for(format!("lab-{model}"), cx))
                                    .on_change(move |v, _, cx| {
                                        e.update(cx, |s, cx| {
                                            if *v {
                                                s.lab_selection.insert(model.into());
                                            } else {
                                                s.lab_selection.remove(model);
                                            }
                                            cx.notify();
                                        })
                                    }),
                            )
                            .into_any_element(),
                    );
                }
                if r.lab_running {
                    text!(
                        r.lab_progress
                            .map(|p| format!("Running · {p}%"))
                            .unwrap_or_else(|| "Preparing…".into())
                    );
                    action!("cancel-lab", "Cancel run", Action::CancelLab, false, false);
                } else {
                    action!(
                        "run-lab",
                        "Run selected models",
                        Action::RunLab {
                            models: self.lab_selection.iter().cloned().collect()
                        },
                        false,
                        self.lab_selection.is_empty() || r.recording
                    );
                }
                for (i, report) in r.reports.iter().enumerate() {
                    text!(report.clone());
                    action!(
                        &format!("export-report-{i}"),
                        "Export report…",
                        Action::ExportReport(i),
                        false,
                        false
                    );
                }
                if !r.reports.is_empty() {
                    action!(
                        "clear-reports",
                        "Clear saved reports…",
                        Action::ClearReports,
                        true,
                        r.lab_running
                    );
                }
                if r.startup_running {
                    text!("Microphone startup test in progress…");
                    action!(
                        "cancel-microphone-startup",
                        "Cancel microphone startup test",
                        Action::CancelMicrophoneStartup,
                        false,
                        false
                    );
                } else {
                    action!(
                        "microphone-startup",
                        "Test microphone startup…",
                        Action::MicrophoneStartup,
                        false,
                        r.recording || r.lab_running
                    );
                }
            }
        }
        if self.confirmation.is_some() {
            let e = cx.entity();
            out.push(
                Button::new("cancel-confirmation", "Cancel confirmation")
                    .with_scroll_parent(Some(&self.scroll))
                    .on_click(move |_, _, cx| {
                        e.update(cx, |s, cx| {
                            s.confirmation = None;
                            cx.notify();
                        })
                    })
                    .into_any_element(),
            );
        }
        let _ = w;
        out
    }
}
