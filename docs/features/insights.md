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

A week with recordings but no usable speed shows “No speed data for these
days.” in the Speed plot. Its seven day targets remain available for exact
recording counts and no-data labels; a week without recordings keeps the
dictation invitation.

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

## Daily word goal

An opt-in **Daily word goal** card sits between the headline figures and the
charts. It is off by default (`dailyWordGoal: 0`); off shows only the picker and
a one-line explanation. The picker offers Off, 250, 500, 1,000, 2,000 and 5,000
words. A hand-edited whole-number goal up to 100,000 stays selectable; anything
else loads as off.

With a goal set, the card reads today's local word bucket and shows a progress
bar (`role="progressbar"`, value clamped to the goal, text such as “312 of 500
words today”), the words still to go or “Goal met”, how many of the last seven
days (today included) met the goal, and a goal streak once it exceeds one day.
An unmet today does not break the streak, matching the recording streak.

The goal reads only the same local daily buckets as the charts. It never
changes recording, transcription, delivery, or history, and nothing about it
leaves the Mac. The bar has no transition, so there is no motion to reduce.
Source: `app/src/lib/dailyGoal.ts` and `app/src/components/home/DailyGoalCard.tsx`.

## Personal bests

An opt-in **Personal bests** card sits below the charts. It is off by
default (`showPersonalBests: false`); off shows only its switch and a one-line
explanation. On, it lists four figures from every retained local daily bucket:
most words in a day, best week (words in a Sunday-to-Saturday calendar week),
most dictations in a day, and the longest run of consecutive days with at least
one recording. Each figure names its date, week start or date range. Ties keep
the earliest day or week. Word figures are omitted when no words were ever
transcribed; with no statistics the card invites a first dictation.

Like the goal it is display only: it reads the same buckets as the charts and
never changes recording, transcription, delivery or history. Reset statistics
clears it. Source: `app/src/lib/personalBests.ts` and
`app/src/components/home/PersonalBestsCard.tsx`.

## Fixtures

`visual-fixtures.html?state=insights` shows real populated components. Additional
states are `insights-speed`, `insights-more`, `insights-empty` (zero persisted
fixture statistics), `insights-gap` (fourth recent day absent), and
`insights-no-speed` (recordings without transcribed words), `insights-goal`
(a 1,000-word goal in progress), `insights-goal-met` (a 500-word goal met), and
`insights-bests` (personal bests on). Append
`appearance=light` or `appearance=dark`; use 1100×780 or 840×620 and scroll the
compact content to its bottom. Screenshot capture requires no CSS/JS layer.
Fixture lifetime totals and daily buckets are independently seeded synthetic
counts; they are not forced to reconcile.

The native pass is pending the integrated redesign build.
