# UI audit: layout and product vocabulary

Issue #877 layout-lane decisions, revalidated against integration base
`fe6a1f4e3277fff9b08bf3244548e4fa8d72c4a4` on 2026-11-03. The audit used an
older dirty 0.43 source tree; it is not evidence that the same defects shipped
in this base. Browser fixtures use synthetic IPC and do not prove native
placement, focus preservation, or persistence.

## Product vocabulary (#895)

Use recognizable destination names, while retaining useful distinctions between
a workspace, a task, and a saved record:

| Term | Intended scope |
| --- | --- |
| Notetaker | Main workspace for capturing, finding, and reviewing meetings. A **meeting** is a saved session; **Meetings** is the settings category for capture/audio behavior. These are intentional umbrella/task names, not competing labels for the same destination. |
| Recording | Settings destination for the recording trigger and listening behavior. **Dictation** names the voice-to-text task, not a separate settings page. Onboarding must direct shortcut changes to **Recording settings**. The current base already does so (the #850 copy work). |
| App overrides | Customize destination for per-app behavior. The internal `styles` ID remains a compatibility detail, not user-facing terminology. A **writing style** can describe an output preference, but help must not direct users to an absent “Styles” destination. |
| Modes | Named bundles of dictation behavior that can be selected or bound to an app. A mode is not the delivery destination. |
| Delivery | Where/how the produced text is delivered (clipboard, auto-paste, files). It is not a synonym for a writing style or mode. |
| Voice commands & spoken formatting | Global switch that gates both replacement commands and spoken formatting. Individual task labels may name the specific capability, not only one half of that global switch. |

The Customize hub owns the canonical visible destinations, including **App overrides** and **Modes**, without ordered step numbers. Palette Settings entries
are derived from category labels. This lane groups the palette's default list;
it does not alter destinations, command availability, or search scoring.

## Truthful local scope (#889)

The sidebar says **Dictation stays on this Mac.** Onboarding and About now
name local dictation, saved history, and retained meeting audio. This removal
retires the provider connection and its consent surface.

## Activity calendar (#898)

**Decision: retain the existing compact maximum plot size and add persistent
calendar anchors.** The current base already caps the heatmap plot at 16rem wide
(under 224px high at the normal root size), unlike the old audit's roughly 286px
height. Do not reintroduce that large square or shrink usable day targets merely
to make the chart less prominent.

Keep the eight week columns and Sunday-through-Saturday placement. Add a visible
inclusive date range (including years), month markers aligned with week columns,
and weekday labels outside the plot. A compact weekday gutter may reduce the
plot's available width inside a narrow analytics column, but the plot never
grows past its existing cap. Keep semantic theme colors and the existing 0–4
intensity mapping unchanged: this is a sequential single-metric calendar, not a
new categorical palette.

Every day keeps its existing accessible date/word/recording label and its
hover/focus/click exact-value behavior. The new **Daily values** disclosure adds
the same data as a table for reading without color or cell-by-cell interaction;
it is collapsed by default to keep the overview restrained. Bar/line charts and
all analytics remain present. Light/dark browser checks cover 720, 880, and
1120px widths. Native WKWebView/theme checks remain part of integrated acceptance.

## Action-item editor (#888)

**Decision: task text always gets its own full-width, wrapping edit row.** Owner,
due date, and Remove use available pane width, not a breakpoint based on the
whole window. Metadata wraps independently. This avoids an 8rem + 8rem metadata
reservation squeezing the actual task down to a few pixels in a split pane.
Visible field labels supplement the existing unique accessible names. Save and
Cancel retain their existing review-only contract; canonical transcript segments
and source provenance are not editable through this form.

## Existing fixes retained (#886, #887, #897)

- Overlay controls already use a separate status row, a 64pt dropdown, and a
  minimum 241pt expanded surface. The old 152px fallback is not a supported
  current expanded geometry. Notch, measured external, unmeasured fallback, and
  floating-pill fixtures are checked separately. No Rust geometry change is
  warranted. Display placement/nonactivation still needs native verification.
- Diagnostics chips already wrap (`StreamChips`) following #853. Preserve that
  fix and the horizontally scrollable diagnostics tab strip; test all streams
  by pointer and keyboard in the compact embedded workspace.
- Home's former ellipsized styles tip was removed by the #850 flow. The current
  source-owned discovery checklist explains app-bound modes and selected-text
  rewrite without CSS-generated alternate wording. Preserve its full help and
  accessible setup/dismiss actions instead of reviving a redundant tip.

## Default palette groups (#899)

Group only the empty/whitespace-query list by section in first-seen section
order; retain declaration order within each group. This keeps **Start
recording** first, all Recording commands together, and Meeting commands together,
including conditional active-meeting entries. Nonempty searches keep the
existing relevance ranking, tie behavior, and keyword matching. Keyboard
selection and execution consume the displayed order.
