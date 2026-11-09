use super::*;
impl Meetings {
    fn meeting_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _w: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(s) = self.model.snapshot.clone() else {
            return vec![];
        };
        let p = Palette::get(cx);
        range
            .filter_map(|index| s.meetings.get(index).map(|m| (index, m)))
            .map(|(index, meeting)| {
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
                let date = |ms| {
                    chrono::DateTime::from_timestamp_millis(ms)
                        .map(|t| {
                            t.with_timezone(&chrono::Local)
                                .format("%b %e, %Y")
                                .to_string()
                        })
                        .unwrap_or_default()
                };
                let new_day = index == 0
                    || date(s.meetings[index - 1].started_at_ms) != date(meeting.started_at_ms);
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .when(new_day, |d| {
                        d.child(
                            div()
                                .text_size(tokens::MICRO_SIZE)
                                .text_color(p.muted)
                                .pt(px(12.))
                                .pb(px(4.))
                                .child(date(meeting.started_at_ms).to_uppercase()),
                        )
                    })
                    .child({
                        let focus = focus.clone();
                        let list = self.list_scroll.clone();
                        let outer = self.content_scroll.clone();
                        canvas(
                            move |bounds, w, _| {
                                let entered = focus.is_focused(w) && !reveal.replace(true);
                                if !focus.is_focused(w) {
                                    reveal.set(false);
                                }
                                if entered {
                                    let list = list.clone();
                                    let outer = outer.clone();
                                    w.on_next_frame(move |w, _| {
                                        let viewport = list.viewport_bounds();
                                        let bounds = list.bounds_for_item(index).unwrap_or(bounds);
                                        if bounds.top() < viewport.top()
                                            || bounds.bottom() > viewport.bottom()
                                        {
                                            list.scroll_to_reveal_item(index);
                                        }
                                        crate::data_components::reveal_bounds(bounds, [&outer]);
                                        w.refresh();
                                    });
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full()
                    })
                    .child(
                        Button::new(format!("meeting-{id}"), title.clone())
                            .nav_row()
                            .toggled(selected)
                            .when(selected, |b| b.bordered())
                            .padding_y(px(5.))
                            .palette(Palette {
                                tint_raised: p.surface.blend(p.accent.opacity(0.06)),
                                ..p
                            })
                            .content(
                                div()
                                    .w_full()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .child(title),
                                            )
                                            .when_some(meeting.status_badge(), |d, badge| {
                                                d.child(
                                                    div()
                                                        .flex_shrink_0()
                                                        .rounded(tokens::CONTROL_RADIUS)
                                                        .px(px(6.))
                                                        .text_size(tokens::MICRO_SIZE)
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(p.text)
                                                        .bg(if meeting.status == "failed" {
                                                            p.danger.opacity(0.14)
                                                        } else {
                                                            p.muted.opacity(0.14)
                                                        })
                                                        .child(badge),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_size(tokens::MICRO_SIZE)
                                                    .text_color(p.muted)
                                                    .child(
                                                        chrono::DateTime::from_timestamp_millis(
                                                            meeting.started_at_ms,
                                                        )
                                                        .map(|t| {
                                                            t.with_timezone(&chrono::Local)
                                                                .format("%-I:%M %p")
                                                                .to_string()
                                                        })
                                                        .unwrap_or_default(),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(tokens::CAPTION_SIZE)
                                                    .line_height(tokens::CAPTION_LEADING)
                                                    .text_color(p.muted)
                                                    .child(meeting.preview.clone()),
                                            )
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_size(tokens::MICRO_SIZE)
                                                    .text_color(p.muted)
                                                    .child(duration(meeting.duration_ms)),
                                            ),
                                    )
                                    .when_some(meeting.matches_line(), |d, line| {
                                        d.child(
                                            div()
                                                .text_size(tokens::MICRO_SIZE)
                                                .line_height(tokens::CAPTION_LEADING)
                                                .text_color(p.accent)
                                                .child(line),
                                        )
                                    }),
                            )
                            .track_focus(&focus)
                            .on_click(cx.listener(move |s, _, _, cx| s.select(id.clone(), cx))),
                    )
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
            content = content.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
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
                            .font_weight(FontWeight::MEDIUM)
                            .child(
                                d.session
                                    .title
                                    .clone()
                                    .unwrap_or_else(|| "Untitled meeting".into()),
                            ),
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
                        .pill()
                        .disabled(disabled)
                        .on_click(cx.listener(|s, _, w, cx| s.name(w, cx))),
                    ),
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
                .child(
                    self.button(
                        "calendar",
                        "Name from calendar…",
                        Action::CalendarLookup(id.clone()),
                        cx,
                    )
                    .pill(),
                )
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
                    .icon_only(crate::icons::Glyph::Trash)
                    .pill()
                    .palette(Palette {
                        text: if confirming { p.danger } else { p.muted },
                        ..p
                    })
                    .disabled(self.model.pending || !self.allowed(&Action::Delete(id.clone())))
                    .on_click(cx.listener(|s, _, _, cx| s.delete(false, cx))),
                ),
        );
        content = content.child(
            div()
                .text_size(tokens::CAPTION_SIZE)
                .text_color(p.muted)
                .child(format!(
                    "{} · {} · {}",
                    local_datetime(d.session.started_at_ms),
                    duration(d.session.duration_ms),
                    if d.session.retain_audio {
                        "Audio kept"
                    } else {
                        "Transcript only"
                    }
                )),
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
            content=content.child(div().p(px(10.)).rounded(tokens::CARD_RADIUS).bg(p.surface_container_low).text_size(tokens::CAPTION_SIZE).text_color(p.muted).child("Audio was not retained for this meeting. Enable Keep Meeting Audio in Meetings settings to play future sessions."));
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
                    content = content.child(
                        div().id("meeting-playback").role(Role::Group).aria_label("Meeting audio playback")
                            .border_1().border_color(p.hairline).rounded(tokens::CARD_RADIUS).bg(p.tint_raised)
                            .px(px(12.)).py(px(8.))
                            .child(div().flex().items_center().gap(px(8.))
                                .child(self.button("audio-toggle", if playing { "Pause" } else if s.audio.state == AudioState::Paused { "Resume" } else { "Play" },
                                    Action::Playback { id: id.clone(), action: if playing { PlaybackAction::Pause } else { PlaybackAction::Play } }, cx)
                                    .icon_only(if playing { crate::icons::Glyph::Pause } else { crate::icons::Glyph::Play })
                                    .size(ButtonSize::Standard).intent(crate::ButtonIntent::Primary).pill())
                                .child(div().flex_1().min_w(px(16.)).child(self.detail_control(&self.seek, w, cx)))
                                .child(div().flex_shrink_0().font_family(tokens::MONO_FONT).text_size(tokens::MICRO_SIZE)
                                    .text_color(p.muted).child(format!("{} / {}", timestamp(s.audio.position_ms), timestamp(s.audio.duration_ms))))
                                .child(self.detail_control(&self.channel_control, w, cx)))
                            .when(s.busy(), |v| v.child(Self::notice("Playback is paused while Murmur records or processes a request.", false)))
                            .when(s.audio.state == AudioState::Buffering, |v| v.child(Self::notice("Buffering retained audio…", false)))
                    );
                }
            }
        }
        let mut exports = div()
            .id("meeting-exports")
            .role(Role::Group)
            .aria_label("Copy or export")
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .items_center()
            .child("Format")
            .child(self.detail_control(&self.format_control, w, cx))
            .child(
                self.button(
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
                )
                .size(ButtonSize::Compact)
                .pill(),
            )
            .child(
                self.button(
                    "export",
                    "Export…",
                    Action::Export {
                        id: id.clone(),
                        format: self.format,
                    },
                    cx,
                )
                .size(ButtonSize::Compact)
                .pill(),
            );
        if d.session.retain_audio {
            exports = exports.child(
                self.button(
                    "bundle",
                    "Export with audio…",
                    Action::BundlePreview(id.clone()),
                    cx,
                )
                .size(ButtonSize::Compact)
                .pill(),
            );
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
            content=content.child(Self::notice(if d.finished(){"One caption per recorded speech segment, with saved speaker names. Untranscribed sections are marked. Review notes are not included."}else{"Stop this meeting before exporting captions."},false).rounded(tokens::CONTROL_RADIUS).bg(p.surface_container_low).px(px(10.)).py(px(8.)).text_size(tokens::CAPTION_SIZE).line_height(tokens::CAPTION_LEADING).text_color(p.muted));
        }
        content = content.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .items_center()
                .pt(px(16.))
                .border_t_1()
                .border_color(p.hairline)
                .child(div().font_weight(FontWeight::MEDIUM).child("Transcript"))
                .child(
                    div()
                        .text_size(tokens::CAPTION_SIZE)
                        .text_color(p.muted)
                        .child(format!("{} segments", d.segments.len())),
                )
                .child(div().flex_1())
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
                    .pill()
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
                .items_start()
                .gap(px(12.))
                .px(px(10.))
                .py(px(8.))
                .border_b_1()
                .border_color(p.hairline);
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
            let mut time = div()
                .w(px(60.))
                .flex_shrink_0()
                .font_family(tokens::MONO_FONT)
                .text_size(tokens::CAPTION_SIZE)
                .text_color(p.muted);
            if segment.audio_available && d.session.retain_audio {
                let channel = segment.channel;
                let start = segment.start_ms;
                time = time.child(
                    self.detail_button(
                        format!("segment-play-{}", segment.id),
                        format!("Play segment at {}, {canonical} channel", timestamp(start)),
                    )
                    .caption(timestamp(start))
                    .icon(crate::icons::Glyph::Play)
                    .padding_x(px(0.))
                    .palette(Palette {
                        text: p.accent,
                        ..p
                    })
                    .intent(crate::ButtonIntent::Ghost)
                    .size(ButtonSize::ExtraSmall)
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
                    .on_click(cx.listener(move |s, _, _, cx| s.play_segment(channel, start, cx))),
                );
            } else {
                time = time.child(timestamp(segment.start_ms));
            }
            row = row
                .child(time)
                .child(
                    div()
                        .w(px(88.))
                        .flex_shrink_0()
                        .text_size(tokens::CAPTION_SIZE)
                        .text_color(if segment.channel == Channel::Me {
                            p.accent
                        } else {
                            p.success
                        })
                        .child(format!(
                            "{display}{}",
                            if display != canonical {
                                format!(" ({canonical})")
                            } else {
                                String::new()
                            }
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(tokens::LABEL_SIZE)
                        .child(gpui::text!(
                            id = ("meeting-segment-text", segment.id),
                            text.chars()
                                .skip(index * 2048)
                                .take(2048)
                                .collect::<String>()
                        )),
                );
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
                state.set(false);
            }
            present
        });
        let p = Palette::get(cx);
        let snapshot = self.model.snapshot.clone();
        let header = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .pt(px(if self.embedded { 16. } else { 32. }))
            .px(px(16.))
            .pb(px(12.))
            .flex_shrink_0()
            .child(
                self.button("back", "Back", Action::Back, cx)
                    .caption("Back to Home")
                    .icon(crate::icons::Glyph::ChevronLeft)
                    .intent(crate::ButtonIntent::Ghost),
            )
            .child(
                div()
                    .id("notetaker-heading")
                    .role(Role::Heading)
                    .aria_label("Notetaker")
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(20.))
                    .child("Notetaker")
                    .child(
                        div()
                            .text_size(tokens::CAPTION_SIZE)
                            .font_weight(FontWeight::NORMAL)
                            .text_color(p.muted)
                            .child("Local meeting transcripts with speaker separation"),
                    ),
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
            root=root.child(div().px(px(16.)).pb(px(12.)).flex().flex_col().flex_shrink_0().gap(px(8.))
                .child(div().flex().items_center().justify_between().gap(px(12.))
                    .child(div().flex().flex_col().gap(px(4.))
                        .child(div().flex().items_center().gap(px(8.)).child(div().size(px(8.)).rounded_full().bg(p.success)).child(Self::notice(format!("{}{}",s.phase.label(),if s.elapsed_ms>0 {format!(" · {}",timestamp(s.elapsed_ms))}else{String::new()}),false)))
                        .child(div().text_size(tokens::MICRO_SIZE).text_color(p.muted).child(format!("{} Microphone · Me    {} System Audio · Them",if s.microphone_active{"●"}else{"○"},if s.system_audio_active{"●"}else{"○"})).id("meeting-channel-help").tooltip(|w,cx|gpui_component::tooltip::Tooltip::new("Me comes from your microphone; Them comes from Mac playback. If your own voice is played through speakers, it can also appear as Them.").build(w,cx))))
                    .child(Button::new("meeting-record",if s.phase.active(){"Stop meeting"}else if s.phase==Phase::Processing{"Finishing…"}else{"Start meeting"}).intent(crate::ButtonIntent::Primary).with_scroll_parent(Some(&self.content_scroll)).disabled(action_disabled).on_click(cx.listener(|s,_,_,cx|s.record(cx)))))
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
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div().flex_1().min_w_0().child(
                                TextField::new(&self.search, "Search meetings")
                                    .search_chrome()
                                    .search_radius(px(999.)),
                            ),
                        )
                        .child(
                            Button::new("meeting-more", "More meeting actions")
                                .icon_only(crate::icons::Glyph::More)
                                .intent(crate::ButtonIntent::Ghost)
                                .size(ButtonSize::Compact)
                                .on_click(cx.listener(|s, _, _, cx| {
                                    s.list_actions_open = !s.list_actions_open;
                                    cx.notify();
                                })),
                        ),
                )
                .when(self.list_actions_open, |v| {
                    v.child(
                        Button::new(
                            "delete-all",
                            if confirm {
                                "Click again to delete all meetings"
                            } else {
                                "Delete all meetings…"
                            },
                        )
                        .size(ButtonSize::Compact)
                        .pill()
                        .palette(Palette {
                            text: if confirm { p.danger } else { p.muted },
                            ..p
                        })
                        .with_scroll_parent(Some(&self.content_scroll))
                        .disabled(s.phase.busy() || self.model.pending || s.meetings.is_empty())
                        .on_click(cx.listener(|s, _, _, cx| s.delete(true, cx))),
                    )
                });
            if s.meetings.is_empty() {
                list=list.child(if s.query.is_empty(){"No meeting transcripts yet. Start one to capture microphone and Mac playback as separate speakers."}else{"No meetings match this search. Try another word or clear the search."});
            }
            // The matched-fields line changes a row's height, so a row that gains or
            // loses it must be measured again like a new row.
            let identity: Vec<_> = s
                .meetings
                .iter()
                .map(|m| {
                    (
                        m.id.clone(),
                        m.started_at_ms,
                        m.search_matches
                            .iter()
                            .any(|field| search_field_label(field).is_some()),
                    )
                })
                .collect();
            if identity != self.list_identity {
                let prefix = identity.starts_with(&self.list_identity);
                let from = if prefix { self.list_identity.len() } else { 0 };
                let handles: Vec<_> = identity
                    .iter()
                    .skip(from)
                    .map(|(id, _, _)| Some(self.focus_for(&format!("meeting-{id}"), cx)))
                    .collect();
                self.list_scroll
                    .splice_focusable(from..self.list_identity.len(), handles);
                self.list_identity = identity;
            }
            list = list.child(div().flex_1().min_h(px(0.)).overflow_hidden().child({
                let entity = cx.entity();
                gpui::list(self.list_scroll.clone(), move |index, w, cx| {
                    entity.update(cx, |s, cx| {
                        s.meeting_rows(index..index + 1, w, cx).pop().unwrap()
                    })
                })
                .size_full()
            }));
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
                            .when(!narrow, |v| {
                                v.w(px(272.)).border_r_1().border_color(p.hairline)
                            })
                            // Includes toolbar/status chrome and a complete meeting row.
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
