use super::*;
impl Meetings {
    fn meeting_rows(
        &mut self,
        range: std::ops::Range<usize>,
        w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(s) = self.model.snapshot.clone() else {
            return vec![];
        };
        let p = Palette::get(cx);
        let scroll_parent = self.list_scroll.0.borrow().base_handle.clone();
        range
            .filter_map(|index| s.meetings.get(index))
            .map(|meeting| {
                let id = meeting.id.clone();
                let selected = self
                    .list_selection
                    .as_ref()
                    .or_else(|| s.detail.as_ref().map(|d| &d.session.id))
                    == Some(&id);
                let focus = self.focus_for(&format!("meeting-{id}"), cx);
                let reveal = self.row_reveal.entry(id.clone()).or_default().clone();
                let title = meeting
                    .title
                    .clone()
                    .unwrap_or_else(|| "Untitled meeting".into());
                div()
                    .relative()
                    .h(px(112.))
                    .p(tokens::FOCUS_WIDTH + px(1.))
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .overflow_hidden()
                    // The title is only 32pt tall. Reveal the complete row on
                    // every focus entry, including Tab and AX Focus.
                    .child(crate::data_components::focus_reveal(
                        focus.clone(),
                        [scroll_parent.clone(), self.content_scroll.clone()],
                        Some(reveal),
                        w,
                        cx,
                    ))
                    .child(
                        Button::new(format!("meeting-{id}"), title)
                            .track_focus(&focus)
                            .toggled(selected)
                            .on_click(cx.listener(move |s, _, _, cx| s.select(id.clone(), cx))),
                    )
                    .child(div().text_size(px(12.)).text_color(p.muted).child(format!(
                        "{} · {}{}",
                        duration(meeting.duration_ms),
                        meeting.status,
                        if meeting.search_matches.is_empty() {
                            String::new()
                        } else {
                            format!(" · Matches: {}", meeting.search_matches.join(", "))
                        }
                    )))
                    .child(
                        chrono::DateTime::from_timestamp_millis(meeting.started_at_ms)
                            .map(|t| {
                                t.with_timezone(&chrono::Local)
                                    .format("%b %e · %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_default(),
                    )
                    .child(meeting.preview.chars().take(160).collect::<String>())
                    .into_any_element()
            })
            .collect()
    }
    fn field(
        &self,
        key: &str,
        label: &str,
        disabled: bool,
        w: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(label.to_owned())
            .when_some(self.fields.get(key), |d, s| {
                d.relative()
                    .child(crate::data_components::focus_reveal(
                        s.read(cx).focus_handle(cx),
                        [self.detail_scroll.clone(), self.content_scroll.clone()],
                        None,
                        w,
                        cx,
                    ))
                    .child(TextField::new(s, label.to_owned()).disabled(disabled))
            })
    }
    // Own one reveal through both viewports so controls do not scroll the
    // inner viewport twice on the same focus entry.
    fn detail_control<V: Render + Focusable + 'static>(
        &self,
        entity: &Entity<V>,
        w: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement + use<V> {
        div()
            .relative()
            .child(crate::data_components::focus_reveal(
                entity.read(cx).focus_handle(cx),
                [self.detail_scroll.clone(), self.content_scroll.clone()],
                None,
                w,
                cx,
            ))
            .child(entity.clone())
    }
    fn notice(text: impl Into<SharedString>, alert: bool) -> Stateful<Div> {
        let text = text.into();
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hash);
        div()
            .id(("meeting-notice", hash.finish()))
            .role(if alert { Role::Alert } else { Role::Status })
            .aria_label(text.clone())
            .child(text)
    }
    fn detail(&mut self, s: &Snapshot, w: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(d) = &s.detail else {
            return div()
                .p(px(24.))
                .child("Select a meeting")
                .child(div().child(
                    "Durable Me/Them transcripts are searchable and stay local to this Mac.",
                ))
                .into_any_element();
        };
        self.ensure_fields(w, cx);
        let id = &d.session.id;
        let p = Palette::get(cx);
        let disabled = s.busy() || self.model.pending;
        let mut content = div().flex().flex_col().gap(px(12.)).p(px(16.));
        if self.naming {
            content = content.child(
                div()
                    .id("meeting-metadata")
                    .role(Role::Group)
                    .aria_label("Name meeting")
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(self.field("title", "Meeting title", disabled, w, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child("Attendees, one per line")
                            .when_some(self.attendees.as_ref(), |v, state| {
                                v.relative()
                                    .child(crate::data_components::focus_reveal(
                                        state.read(cx).focus_handle(cx),
                                        [self.detail_scroll.clone(), self.content_scroll.clone()],
                                        None,
                                        w,
                                        cx,
                                    ))
                                    .child(
                                        gpui_component::input::Textarea::new(state)
                                            .aria_label("Attendees, one per line")
                                            .disabled(disabled)
                                            .h(px(64.)),
                                    )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                self.detail_button("save-details", "Save details")
                                    .size(ButtonSize::Compact)
                                    .disabled(disabled)
                                    .on_click(cx.listener(|s, _, _, cx| s.save_metadata(cx))),
                            )
                            .child(
                                self.detail_button("cancel-name", "Cancel naming")
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|s, _, w, cx| {
                                        s.naming = false;
                                        s.clear_fields();
                                        if let Some(f) = s.focus.get("rename") {
                                            w.focus(f, cx);
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
        } else {
            content = content
                .child(
                    div()
                        .id("meeting-title")
                        .role(Role::Heading)
                        .aria_label(
                            d.session
                                .title
                                .clone()
                                .unwrap_or_else(|| "Untitled meeting".into()),
                        )
                        .text_size(px(18.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(
                            d.session
                                .title
                                .clone()
                                .unwrap_or_else(|| "Untitled meeting".into()),
                        ),
                )
                .child(div().text_size(px(12.)).text_color(p.muted).child(format!(
                    "{} · {} · {}",
                    local_datetime(d.session.started_at_ms),
                    duration(d.session.duration_ms),
                    if d.session.retain_audio {
                        "Audio kept"
                    } else {
                        "Transcript only"
                    }
                )))
                .when(
                    !d.session.attendees.is_empty() || d.session.title_source.is_some(),
                    |v| {
                        v.child(format!(
                            "{}{}",
                            if d.session.attendees.is_empty() {
                                String::new()
                            } else {
                                format!("Attendees: {} · ", d.session.attendees.join(", "))
                            },
                            match d.session.title_source.as_deref() {
                                Some("calendar") => "Named from calendar",
                                Some("generated") => "Generated title",
                                Some("manual") => "Manually named",
                                _ => "",
                            }
                        ))
                    },
                )
                .child(
                    self.detail_button(
                        "rename",
                        if d.session.title.is_some() {
                            "Rename"
                        } else {
                            "Name meeting"
                        },
                    )
                    .track_focus(&self.focus_for("rename", cx))
                    .size(ButtonSize::Compact)
                    .disabled(disabled)
                    .on_click(cx.listener(|s, _, w, cx| s.name(w, cx))),
                );
        }
        let confirming = self
            .confirmation
            .as_ref()
            .is_some_and(|(key, end)| key == id && Instant::now() < *end);
        content = content.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .child(self.button(
                    "calendar",
                    "Name from calendar…",
                    Action::CalendarLookup(id.clone()),
                    cx,
                ))
                .child(
                    self.detail_button(
                        "delete-meeting",
                        if confirming {
                            "Confirm delete…"
                        } else {
                            "Delete meeting…"
                        },
                    )
                    .size(ButtonSize::Compact)
                    .intent(crate::ButtonIntent::Danger)
                    .disabled(self.model.pending || !self.allowed(&Action::Delete(id.clone())))
                    .on_click(cx.listener(|s, _, _, cx| s.delete(false, cx))),
                ),
        );
        match &s.calendar {
            Calendar::Choosing(events) => {
                let mut picker = div()
                    .id("calendar-picker")
                    .role(Role::Group)
                    .aria_label("Choose an overlapping event")
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child("Choose an overlapping event")
                    .child(
                        "Only the title and attendee names of the event you apply will be saved.",
                    );
                if events.is_empty() {
                    picker = picker.child(
                        "No calendar events overlap this meeting. You can still name it manually.",
                    );
                }
                for (index, event) in events.iter().enumerate() {
                    let token = event.token.clone();
                    let selected = self.calendar_selection.as_ref() == Some(&token);
                    picker = picker.child(
                        self.detail_button(
                            format!("calendar-event-{index}"),
                            format!(
                                "{} · {} – {} · Attendees: {}",
                                event.title,
                                local_datetime(event.start_ms),
                                local_datetime(event.end_ms),
                                event.attendees.join(", ")
                            ),
                        )
                        .toggled(selected)
                        .disabled(disabled)
                        .on_click(cx.listener(move |s, _, _, cx| {
                            s.calendar_selection = Some(token.clone());
                            cx.notify();
                        })),
                    );
                }
                let token = self.calendar_selection.clone();
                picker = picker.child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            self.button(
                                "calendar-apply",
                                "Apply event",
                                Action::CalendarApply {
                                    id: id.clone(),
                                    token: token.clone().unwrap_or_default(),
                                },
                                cx,
                            )
                            .disabled(disabled || token.is_none()),
                        )
                        .child(
                            self.detail_button("calendar-cancel", "Cancel calendar lookup")
                                .size(ButtonSize::Compact)
                                .on_click(cx.listener(|s, _, w, cx| {
                                    s.close_calendar(cx);
                                    if let Some(f) = s.focus.get("calendar") {
                                        w.focus(f, cx);
                                    }
                                })),
                        ),
                );
                content = content.child(picker);
            }
            Calendar::Unavailable {
                message,
                denied,
                unsupported,
            } => {
                let mut fallback = div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(Self::notice(*message, false))
                    .child(
                        self.detail_button("manual-name", "Name manually")
                            .on_click(cx.listener(|s, _, w, cx| s.name(w, cx))),
                    );
                if !unsupported {
                    fallback = fallback.child(self.button(
                        "calendar-settings",
                        "Open Calendar settings…",
                        Action::CalendarSettings,
                        cx,
                    ));
                }
                if *denied {
                    fallback = fallback.child(self.button(
                        "calendar-reset",
                        "Reset Calendar access…",
                        Action::CalendarReset,
                        cx,
                    ));
                }
                content = content.child(fallback);
            }
            Calendar::Closed => (),
        }
        if !d.session.retain_audio {
            content=content.child("Audio was not retained for this meeting. Enable Keep Meeting Audio in Meetings settings to play future sessions.");
        } else {
            match s.audio.state {
                AudioState::Loading => {
                    content = content.child(Self::notice("Loading retained meeting audio…", false))
                }
                AudioState::NotRetained => {
                    content = content.child("Audio was not retained for this meeting.")
                }
                AudioState::NotFinished => {
                    content =
                        content.child("Playback will be available after this meeting finishes.")
                }
                AudioState::NoAudio => {
                    content = content.child("No retained audio is available for this meeting.")
                }
                AudioState::Error => {
                    content = content
                        .child(Self::notice(
                            "Retained meeting audio could not be loaded.",
                            true,
                        ))
                        .child(self.button(
                            "retry-audio",
                            "Retry audio access",
                            Action::Playback {
                                id: id.clone(),
                                action: PlaybackAction::Retry,
                            },
                            cx,
                        ))
                }
                _ => {
                    let playing =
                        matches!(s.audio.state, AudioState::Playing | AudioState::Buffering);
                    let state = (
                        s.audio.duration_ms,
                        s.audio.position_ms,
                        disabled || s.audio.state == AudioState::Buffering,
                    );
                    if self.seek_state != Some(state) {
                        self.seek_state = Some(state);
                        self.seek.update(cx, |range, cx| {
                            range.set_max(state.0.min(i64::MAX as u64) as i64, cx);
                            range.set_playback_position(state.1 as i64, state.2, w, cx);
                        });
                    }
                    self.channel_control.update(cx, |c, cx| {
                        c.set_disabled(disabled || s.audio.state == AudioState::Buffering, cx);
                        if c.value()
                            .is_none_or(|v| v.as_ref() != s.audio.channel.wire())
                        {
                            c.set_value(s.audio.channel.wire(), cx);
                        }
                    });
                    content=content.child(div().id("meeting-playback").role(Role::Group).aria_label("Meeting audio playback").flex().flex_col().gap(px(8.))
                    .child(div().flex().flex_wrap().items_center().gap(px(8.))
                        .child(self.button("audio-toggle",if playing{"Pause"}else if s.audio.state==AudioState::Paused{"Resume"}else{"Play"},Action::Playback{id:id.clone(),action:if playing{PlaybackAction::Pause}else{PlaybackAction::Play}},cx))
                        .child(format!("{} / {}",timestamp(s.audio.position_ms),timestamp(s.audio.duration_ms))).child(self.detail_control(&self.channel_control, w, cx)))
                    .child(self.detail_control(&self.seek, w, cx))
                    .when(s.busy(),|v|v.child(Self::notice("Playback is paused while Murmur records or processes a request.",false)))
                    .when(s.audio.state==AudioState::Buffering,|v|v.child(Self::notice("Buffering retained audio…",false))));
                }
            }
        }
        let mut exports = div()
            .id("meeting-exports")
            .role(Role::Group)
            .aria_label("Copy or export")
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .items_center()
            .child("Format")
            .child(self.detail_control(&self.format_control, w, cx))
            .child(self.button(
                "copy",
                if self.format.captions() {
                    "Copy captions"
                } else {
                    "Copy transcript"
                },
                Action::Copy {
                    id: id.clone(),
                    format: self.format,
                },
                cx,
            ))
            .child(self.button(
                "export",
                "Export…",
                Action::Export {
                    id: id.clone(),
                    format: self.format,
                },
                cx,
            ));
        if d.session.retain_audio {
            exports = exports.child(self.button(
                "bundle",
                "Export with audio…",
                Action::BundlePreview(id.clone()),
                cx,
            ));
        }
        content = content.child(exports);
        if let Some(b) = &s.bundle {
            content=content.child(div().id("bundle-preview").role(Role::Group).aria_label("Export with audio").flex().flex_col().gap(px(8.))
            .child(format!("The .zip holds the {} as {}, {} audio segments ({} KB), and a manifest. Audio is saved exactly as recorded.",if self.format.captions(){"captions"}else{"transcript"},self.format.wire(),b.audio_segments,b.audio_bytes.div_ceil(1024)))
            .when(b.unavailable>0,|v|v.child(Self::notice(format!("{} retained audio segments are missing and will be left out. The manifest lists them, so the bundle is incomplete.",b.unavailable),true)))
            .when(b.audio_bytes>b.max_bytes,|v|v.child(Self::notice("This is over the bundle limit. Use Export… for the transcript on its own.",true)))
            .child(self.button("bundle-save","Choose location…",Action::BundleExport{id:id.clone(),format:self.format},cx))
            .child(self.detail_button("cancel-bundle","Cancel").size(ButtonSize::Compact).on_click(cx.listener(|s,_,_,cx|{if let Some(s)=&mut s.model.snapshot{s.bundle=None;}cx.notify();}))));
        }
        if self.format.captions() {
            content=content.child(Self::notice(if d.finished(){"One caption per recorded speech segment, with saved speaker names. Untranscribed sections are marked. Review notes are not included."}else{"Stop this meeting before exporting captions."},false));
        }
        content = content.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .child(format!("Transcript · {} segments", d.segments.len()))
                .child(
                    self.detail_button(
                        "speakers",
                        format!(
                            "Speakers: {}, {}{}",
                            d.me,
                            d.them,
                            if d.remote.is_empty() {
                                String::new()
                            } else {
                                format!(
                                    ", {}",
                                    d.remote
                                        .iter()
                                        .map(|s| s.label.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            }
                        ),
                    )
                    .size(ButtonSize::Compact)
                    .on_click(cx.listener(|s, _, _, cx| {
                        s.speakers = !s.speakers;
                        cx.notify();
                    })),
                ),
        );
        if self.speakers {
            let mut labels = div()
                .id("speaker-labels")
                .role(Role::Group)
                .aria_label("Speaker labels")
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(self.field("me", "Me speaker label", self.model.pending, w, cx))
                .child(self.field("them", "Them speaker label", self.model.pending, w, cx))
                .child(self.button(
                    "save-labels",
                    "Save labels",
                    Action::SaveLabels {
                        id: id.clone(),
                        revision: d.review_revision,
                        me: self.value("me", cx),
                        them: self.value("them", cx),
                    },
                    cx,
                ));
            if !d.remote.is_empty() {
                labels = labels.child("Remote speakers").child(format!(
                    "Names apply only to this meeting. Uncertain passages remain {}.",
                    d.them
                ));
            }
            for speaker in &d.remote {
                let key = format!("remote-{}", speaker.id);
                labels = labels
                    .child(self.field(
                        &key,
                        &format!("Remote speaker {} label", speaker.id),
                        self.model.pending,
                        w,
                        cx,
                    ))
                    .child(self.button(
                        format!("save-{key}"),
                        format!("Save Speaker {}", speaker.id),
                        Action::RenameSpeaker {
                            id: id.clone(),
                            speaker: speaker.id,
                            label: self.value(&key, cx),
                        },
                        cx,
                    ));
            }
            content = content.child(labels);
        }
        let pages = d.segments.len().div_ceil(24).max(1);
        self.page = self.page.min(pages - 1);
        if d.segments.is_empty() {
            content = content.child("No speech segments were saved.");
        }
        for segment in d.segments.iter().skip(self.page * 24).take(24) {
            let canonical = if segment.channel == Channel::Me {
                "Me"
            } else {
                "Them"
            };
            let display = d.label(segment);
            let mut row = div()
                .id(("meeting-segment", segment.id))
                .role(Role::Article)
                .aria_label(format!(
                    "{canonical} channel, {display}, at {}",
                    timestamp(segment.start_ms)
                ))
                .flex()
                .flex_col()
                .gap(px(4.))
                .py(px(8.))
                .border_b_1()
                .border_color(p.border)
                .child(format!(
                    "{} · {}{}",
                    timestamp(segment.start_ms),
                    display,
                    if display != canonical {
                        format!(" ({canonical})")
                    } else {
                        String::new()
                    }
                ));
            let text = if segment.status == "final" {
                segment.text.as_str()
            } else if segment.status == "pending" {
                "Transcript pending…"
            } else {
                "Transcription failed."
            };
            let pieces = text.chars().count().div_ceil(2048).max(1);
            let index = *self.text_pages.entry(segment.id).or_default();
            let index = index.min(pieces - 1);
            row = row.child(gpui::text!(
                id = ("meeting-segment-text", segment.id),
                text.chars()
                    .skip(index * 2048)
                    .take(2048)
                    .collect::<String>()
            ));
            if segment.audio_available && d.session.retain_audio {
                let action = PlaybackAction::Seek(segment.start_ms);
                let channel = segment.channel;
                let start = segment.start_ms;
                row = row.child(
                    self.detail_button(
                        format!("segment-play-{}", segment.id),
                        format!("Play segment at {}, {canonical} channel", timestamp(start)),
                    )
                    .size(ButtonSize::Compact)
                    .disabled(
                        disabled
                            || matches!(
                                s.audio.state,
                                AudioState::Error
                                    | AudioState::Loading
                                    | AudioState::Buffering
                                    | AudioState::NoAudio
                                    | AudioState::NotFinished
                            ),
                    )
                    .on_click(cx.listener(move |s, _, _, cx| {
                        // One atomic host intent: channel plus seek plus play, rather
                        // than racing three independent asynchronous operations.
                        s.play_segment(channel, start, cx);
                    })),
                );
                let _ = action;
            }
            if pieces > 1 {
                let key = segment.id;
                row = row.child(
                    self.detail_button(
                        format!("segment-text-{key}"),
                        format!("Transcript text page {} of {pieces} · Next", index + 1),
                    )
                    .on_click(cx.listener(move |s, _, _, cx| {
                        s.text_pages.insert(key, (index + 1) % pieces);
                        cx.notify();
                    })),
                );
            }
            content = content.child(row);
        }
        if pages > 1 {
            content = content.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        self.detail_button("previous-transcript", "Previous transcript page")
                            .disabled(self.page == 0)
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.page = s.page.saturating_sub(1);
                                cx.notify();
                            })),
                    )
                    .child(format!("{} / {pages}", self.page + 1))
                    .child(
                        self.detail_button("next-transcript", "Next transcript page")
                            .disabled(self.page + 1 == pages)
                            .on_click(cx.listener(|s, _, _, cx| {
                                s.page += 1;
                                cx.notify();
                            })),
                    ),
            );
        }
        content.into_any_element()
    }
}
impl Render for Meetings {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.drain(cx);
        // Keep focus-entry state across virtualized remounts. Wheel scrolling
        // while focused must not trigger another reveal; blur resets it even
        // when that row is outside the rendered range.
        self.row_reveal.retain(|id, state| {
            let present = self
                .model
                .snapshot
                .as_ref()
                .is_some_and(|s| s.meetings.iter().any(|m| &m.id == id));
            if !self
                .focus
                .get(&format!("meeting-{id}"))
                .is_some_and(|f| f.is_focused(w))
            {
                state.reset();
            }
            present
        });
        let p = Palette::get(cx);
        let snapshot = self.model.snapshot.clone();
        let header = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .pt(px(32.))
            .px(px(16.))
            .pb(px(12.))
            .flex_shrink_0()
            .child(self.button("back", "Back", Action::Back, cx))
            .child(
                div()
                    .id("notetaker-heading")
                    .role(Role::Heading)
                    .aria_label("Notetaker")
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Notetaker"),
            );
        let mut root = div()
            .id("meetings")
            .capture_key_down(cx.listener(Self::key))
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.content_scroll)
            .bg(p.surface)
            .text_color(p.text)
            .text_size(tokens::BODY_SIZE)
            .line_height(tokens::BODY_LEADING)
            .child(header);
        if let Some(problem) = self.model.problem {
            root = root
                .child(Self::notice(problem.message(), true))
                .child(self.button("retry", "Retry", Action::Read, cx));
        }
        if let Some(s) = snapshot {
            let action_disabled = self.model.pending
                || s.phase == Phase::Stopping
                || s.phase == Phase::Processing
                || (!s.phase.active()
                    && (s.capture_busy || s.permission == Permission::Unsupported));
            root=root.child(div().p(px(16.)).flex().flex_col().flex_shrink_0().gap(px(8.))
                .child(div().flex().flex_wrap().items_center().gap(px(8.)).child(Self::notice(format!("{}{}",s.phase.label(),if s.elapsed_ms>0 {format!(" · {}",timestamp(s.elapsed_ms))}else{String::new()}),false))
                    .child(Button::new("meeting-record",if s.phase.active(){"Stop meeting"}else if s.phase==Phase::Processing{"Finishing…"}else{"Start meeting"}).with_scroll_parent(Some(&self.content_scroll)).disabled(action_disabled).on_click(cx.listener(|s,_,_,cx|s.record(cx)))))
                .child(format!("{}Microphone · Me    {}System Audio · Them",if s.microphone_active{"● "}else{"○ "},if s.system_audio_active{"● "}else{"○ "}))
                .child("Me comes from your microphone; Them comes from Mac playback. If your own voice is played through speakers, it can also appear as Them.")
                .when(!s.busy()&&matches!(s.permission,Permission::Unknown|Permission::Denied),|v|v.child("System Audio access is requested only when you explicitly check it or start a meeting.").child(self.button("check-access","Check access",Action::CheckAccess,cx)))
                .when(s.permission==Permission::Unsupported,|v|v.child(Self::notice(error_message("unsupported_os"),false)))
                .when(s.permission==Permission::Denied,|v|v.child(self.button("audio-settings","Open settings…",Action::AudioSettings,cx)))
                .when(s.needs_relaunch,|v|v.child(Self::notice("Quit and reopen Murmur to finish enabling System Audio access.",false)))
                .when_some(s.failure,|v,error|v.child(Self::notice(error,true)))
                .when_some(s.echo_notice.clone(),|v,text|v.child(Self::notice(text,false)))
                .when_some(s.notice.clone(),|v,text|v.child(Self::notice(text,false))));
            let confirm = self
                .confirmation
                .as_ref()
                .is_some_and(|(key, end)| key == "*" && Instant::now() < *end);
            let mut list = div()
                .id("saved-meetings")
                .role(Role::Group)
                .aria_label("Saved meetings")
                .relative()
                .child(crate::data_components::focus_reveal(
                    self.search.read(cx).focus_handle(cx),
                    Some(self.content_scroll.clone()),
                    None,
                    w,
                    cx,
                ))
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(12.))
                .child(TextField::new(&self.search, "Search meetings"))
                .child(
                    Button::new(
                        "delete-all",
                        if confirm {
                            "Click again to delete all meetings"
                        } else {
                            "Delete all meetings…"
                        },
                    )
                    .size(ButtonSize::Compact)
                    .with_scroll_parent(Some(&self.content_scroll))
                    .disabled(s.phase.busy() || self.model.pending || s.meetings.is_empty())
                    .on_click(cx.listener(|s, _, _, cx| s.delete(true, cx))),
                );
            if s.meetings.is_empty() {
                list=list.child(if s.query.is_empty(){"No meeting transcripts yet. Start one to capture microphone and Mac playback as separate speakers."}else{"No meetings match this search. Try another word or clear the search."});
            }
            list = list.child(
                div().flex_1().min_h(px(0.)).overflow_hidden().child(
                    uniform_list(
                        "saved-meeting-rows",
                        s.meetings.len(),
                        cx.processor(Self::meeting_rows),
                    )
                    .track_scroll(&self.list_scroll)
                    .size_full(),
                ),
            );
            if s.meetings.len() < s.total {
                list = list.child(self.button(
                    "load-more",
                    "Load more meetings",
                    Action::List {
                        query: s.query.clone(),
                        limit: (s.meetings.len() + 50).min(10000),
                    },
                    cx,
                ));
            }
            list = list.child(Self::notice(
                format!("{} meetings shown", s.meetings.len()),
                false,
            ));
            let detail = self.detail(&s, w, cx);
            let narrow = f32::from(w.viewport_size().width) < 768.;
            root = root.child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .when(narrow, |v| {
                        // Taller permission/error chrome scrolls with the page;
                        // it must never consume the detail viewport.
                        v.flex_col().min_h(px(448.)).flex_shrink_0()
                    })
                    .child(
                        div()
                            .id("meeting-list-scroll")
                            .when(!narrow, |v| v.w(px(264.)))
                            // Includes the toolbar/status chrome plus at least one
                            // complete 112pt row and its focus-ring inset.
                            .when(narrow, |v| v.h(px(288.)).flex_shrink_0())
                            .min_h(px(0.))
                            .child(list.h_full()),
                    )
                    .child(
                        div()
                            .id("meeting-detail-scroll")
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(160.))
                            .overflow_y_scroll()
                            .track_scroll(&self.detail_scroll)
                            .child(detail),
                    ),
            );
        } else {
            root = root.child(Self::notice("Loading meetings…", false));
        }
        root
    }
}

pub(super) fn local_datetime(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%a, %b %e, %Y %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
