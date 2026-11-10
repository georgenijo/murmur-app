# Insights

Insights computes usage on this Mac. Its four headline figures show lifetime
words, the average of the retained (up to 100) eligible dictation-speed samples,
lifetime dictations, and estimated lifetime typing time saved. The speed caption
names the actual retained sample count. Time saved compares typing at 40 words
per minute with recorded dictation time and rounds to whole minutes.

Daily activity defaults to Words; Speed selects the same seven local-calendar
days. Words uses zero-based bars and rounds its maximum up to a multiple of 200.
Speed uses a line: its minimum is the previous multiple of 20 strictly below the
smallest observed speed (clamped to zero), and its maximum rounds up to a multiple
of 40. Missing speed or zero recordings leaves a gap, preserving the day's
keyboard target and weekday slot. The accessible chart name states the speed
range. Hover, focus and click show exact values; Escape and blur dismiss them
without moving the plot.

The Last 8 weeks calendar includes its exact date range, month/weekday labels,
five intensity levels and a matching Less/More legend. Levels are relative to
the largest word count in the visible calendar: zero, up to 25%, up to 50%, up
to 75%, and above 75%. Empty cells have an inset boundary; dark activity levels
mix primary over background at 35%, 54%, 75% and 100%. A positive current streak
appears below the legend. All charts expose daily-value tables to assistive
technology without a visible disclosure.

More statistics starts collapsed and includes nonzero total dictation time,
corrections taught and meetings this month. Its two-column grid aligns exactly
with the Daily activity card. Most-used mode and Paste last are omitted. When
there is no recorded usage, the headline shows zero counts and unavailable
speed/time savings, the plot invites dictation, and empty statistics/streaks are
omitted. An inactive recent week also shows the plot invitation while retaining
lifetime totals and any nonzero secondary statistics.

At viewport widths up to 950px, headline figures form a two-by-two grid and the
full-width chart/calendar stack vertically. Only the content column scrolls.
Back to Home remains available and restores Home navigation focus.

## Fixtures

`visual-fixtures.html?state=insights` shows real populated components. Additional
states are `insights-speed`, `insights-more`, `insights-empty` (zero persisted
fixture statistics), and `insights-gap` (fourth recent day absent). Append
`appearance=light` or `appearance=dark`; use 1100×780 or 840×620 and scroll the
compact content to its bottom. Screenshot capture requires no CSS/JS layer.
Fixture lifetime totals and daily buckets are independently seeded synthetic
counts; they are not forced to reconcile.

The native pass is pending the integrated redesign build.
