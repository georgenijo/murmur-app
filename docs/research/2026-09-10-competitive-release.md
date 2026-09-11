# Competitive release audit, September 10, 2026

Compared Murmur v0.45.0 at `c8053e41` with current official product documentation.
The goal was a small release that gives coworkers useful new capabilities while
preserving local transcription and existing app behavior.

## Selected features

| Feature | Competitor evidence | Murmur gap and release scope |
| --- | --- | --- |
| Function-key recording shortcuts | [Wispr Flow](https://docs.wisprflow.ai/articles/4816967992-how-to-use-command-mode) supports F1–F20 on Mac. [FluidVoice](https://fluidvoice.org/docs/getting-started/) lets users choose a global shortcut. | Add F1–F20 to the existing three modifier choices, across Hold Down, Double-Tap, and Both. Preserve the current defaults and independent transform/query keys. |
| Speaker-labeled meeting captions | [MacWhisper](https://www.macwhisper.com/) offers SRT and WebVTT export and speaker recognition. | Export the existing timed meeting segments as SRT or WebVTT, including saved speaker names and explicit transcription gaps. Keep capture times, including overlaps. |
| MP4/MOV transcription | [Superwhisper](https://superwhisper.com/docs/get-started/transcribe-files) supports MP4 transcription. [MacWhisper](https://www.macwhisper.com/) lists MOV and MP4. | Accept these containers through Murmur's existing file queue and choose a supported audio track even when video is first. Keep decoding local. |

Murmur already ships local dictation, multilingual recognition, live provisional
text, app/site Modes, vocabulary, snippets, selected-text rewriting, local meeting
summaries, batch audio-file transcription, sound cues, and Paste Last. Adding
duplicate controls for these would not close a product gap. The comparison used
[Wispr's feature list](https://wisprflow.ai/features),
[FluidVoice's features](https://fluidvoice.org/features/),
[Superwhisper's real-time documentation](https://superwhisper.com/docs/common-issues/realtime),
and [VoiceInk's overview](https://tryvoiceink.com/docs/introduction).

## Scope decisions

- General subtitle export for imported recordings needs timestamp support from
  the dictation backends. This release uses existing meeting segment times and
  makes no word-level alignment claim.
- Mouse activation needs additional macOS event support in the current rdev
  dependency. Function keys already have native mappings and fit the existing
  recording detectors.
- URL automation is useful, as shown by
  [Superwhisper's mode automation](https://superwhisper.com/docs/modes/switching-modes).
  An opt-in switch alone would let any webpage start the microphone afterward.
  A future version needs request authentication or explicit confirmation,
  startup ordering, and verified focus/delivery behavior.
- Raw/Delivered history inspection and reprocessing remain useful later work.
  [Superwhisper's history](https://superwhisper.com/docs/get-started/interface-history)
  exposes both views. Murmur already retains the necessary raw text locally,
  but reprocessing needs a deliberate immutable-source and delivery contract.

Caption serialization follows the
[WebVTT specification](https://www.w3.org/TR/webvtt1/), including overlapping
speech cues, escaped caption text, and millisecond timestamps. These exports
represent captured speech segments rather than newly generated content.
