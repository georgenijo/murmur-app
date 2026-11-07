# AMI ES2004a diarization boundary fixture

`diarization-ami-es2004a.json` contains only the 94 inferred turn boundaries,
quality scores and four opaque speaker integers from the first measured
FluidAudio 0.14.1 pass over `ES2004a.Mix-Headset.wav`. It contains no transcript,
audio, embeddings or personal user data. See
`docs/design/issue-540-spike.md` for the exact dependency pin, model revision,
measurement method and corpus provenance. This is a clean headset mixture,
not the AMI distant-microphone signal.

The AMI corpus is distributed under CC BY 4.0 by the AMI Consortium:
https://groups.inf.ed.ac.uk/ami/corpus/

The regression test divides the public recording into 15-second transcript
chunks and verifies that conservative attribution reaches all four voices,
leaves mixed/uncertain chunks at channel attribution and preserves every
microphone chunk and transcript byte. It tests integration of measured turns
with Murmur's assignment policy. It does not replace rerunning the signed
worker against the original audio or a current production capture smoke.
