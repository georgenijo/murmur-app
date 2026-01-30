/**
 * Audio capture module using Web Audio API
 * Captures microphone audio and produces WAV format suitable for whisper.cpp
 */

const TARGET_SAMPLE_RATE = 16000; // Whisper expects 16kHz
const NUM_CHANNELS = 1; // Mono

interface AudioCaptureState {
  mediaStream: MediaStream | null;
  audioContext: AudioContext | null;
  sourceNode: MediaStreamAudioSourceNode | null;
  processorNode: ScriptProcessorNode | null;
  audioChunks: Float32Array[];
  isRecording: boolean;
  originalSampleRate: number;
}

let state: AudioCaptureState = {
  mediaStream: null,
  audioContext: null,
  sourceNode: null,
  processorNode: null,
  audioChunks: [],
  isRecording: false,
  originalSampleRate: 48000,
};

/**
 * Request microphone permission and start capturing audio
 */
export async function startAudioCapture(): Promise<void> {
  if (state.isRecording) {
    throw new Error('Already recording');
  }

  // Request microphone access - browser handles permission natively
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: {
      channelCount: 1,
      echoCancellation: false,
      noiseSuppression: false, // We do noise reduction in Python
      autoGainControl: true,
    },
  });

  // Create audio context (browser may not honor sample rate request)
  const audioContext = new AudioContext();
  state.originalSampleRate = audioContext.sampleRate;

  const sourceNode = audioContext.createMediaStreamSource(stream);

  // Use ScriptProcessorNode to capture raw PCM
  // Buffer size of 4096 is a good balance between latency and performance
  const processorNode = audioContext.createScriptProcessor(4096, 1, 1);

  state.audioChunks = [];

  processorNode.onaudioprocess = (event) => {
    if (state.isRecording) {
      // Get the raw PCM data from input channel 0
      const inputData = event.inputBuffer.getChannelData(0);
      // Make a copy since the buffer is reused
      state.audioChunks.push(new Float32Array(inputData));
    }
  };

  // Connect the nodes
  sourceNode.connect(processorNode);
  processorNode.connect(audioContext.destination); // Required for processing to work

  state = {
    ...state,
    mediaStream: stream,
    audioContext,
    sourceNode,
    processorNode,
    audioChunks: [],
    isRecording: true,
  };
}

/**
 * Stop recording and return the audio as a base64-encoded WAV string
 */
export async function stopAudioCapture(): Promise<string> {
  if (!state.isRecording) {
    throw new Error('Not recording');
  }

  state.isRecording = false;

  // Disconnect and cleanup audio nodes
  if (state.processorNode) {
    state.processorNode.disconnect();
  }
  if (state.sourceNode) {
    state.sourceNode.disconnect();
  }
  if (state.mediaStream) {
    state.mediaStream.getTracks().forEach(track => track.stop());
  }

  // Concatenate all audio chunks
  const totalLength = state.audioChunks.reduce((acc, chunk) => acc + chunk.length, 0);
  const audioData = new Float32Array(totalLength);
  let offset = 0;
  for (const chunk of state.audioChunks) {
    audioData.set(chunk, offset);
    offset += chunk.length;
  }

  // Close audio context after getting data
  if (state.audioContext) {
    await state.audioContext.close();
  }

  // Resample to 16kHz if needed
  let finalAudioData = audioData;
  if (state.originalSampleRate !== TARGET_SAMPLE_RATE) {
    finalAudioData = await resampleAudio(audioData, state.originalSampleRate, TARGET_SAMPLE_RATE);
  }

  // Convert to WAV
  const wavBuffer = encodeWAV(finalAudioData, TARGET_SAMPLE_RATE);

  // Convert to base64
  const base64 = arrayBufferToBase64(wavBuffer);

  // Reset state
  state = {
    mediaStream: null,
    audioContext: null,
    sourceNode: null,
    processorNode: null,
    audioChunks: [],
    isRecording: false,
    originalSampleRate: 48000,
  };

  return base64;
}

/**
 * Check if currently recording
 */
export function isRecording(): boolean {
  return state.isRecording;
}

/**
 * Resample audio to target sample rate using OfflineAudioContext
 */
async function resampleAudio(
  audioData: Float32Array,
  fromSampleRate: number,
  toSampleRate: number
): Promise<Float32Array> {
  const duration = audioData.length / fromSampleRate;
  const offlineContext = new OfflineAudioContext(
    NUM_CHANNELS,
    Math.ceil(duration * toSampleRate),
    toSampleRate
  );

  const buffer = offlineContext.createBuffer(NUM_CHANNELS, audioData.length, fromSampleRate);
  buffer.getChannelData(0).set(audioData);

  const source = offlineContext.createBufferSource();
  source.buffer = buffer;
  source.connect(offlineContext.destination);
  source.start(0);

  const renderedBuffer = await offlineContext.startRendering();
  return renderedBuffer.getChannelData(0);
}

/**
 * Encode Float32Array audio data as WAV file
 */
function encodeWAV(audioData: Float32Array, sampleRate: number): ArrayBuffer {
  const numChannels = NUM_CHANNELS;
  const bitsPerSample = 16;
  const bytesPerSample = bitsPerSample / 8;
  const blockAlign = numChannels * bytesPerSample;
  const byteRate = sampleRate * blockAlign;
  const dataSize = audioData.length * bytesPerSample;
  const headerSize = 44;
  const totalSize = headerSize + dataSize;

  const buffer = new ArrayBuffer(totalSize);
  const view = new DataView(buffer);

  // WAV header
  writeString(view, 0, 'RIFF');
  view.setUint32(4, totalSize - 8, true); // File size - 8
  writeString(view, 8, 'WAVE');
  writeString(view, 12, 'fmt ');
  view.setUint32(16, 16, true); // Subchunk1Size (16 for PCM)
  view.setUint16(20, 1, true); // AudioFormat (1 = PCM)
  view.setUint16(22, numChannels, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, byteRate, true);
  view.setUint16(32, blockAlign, true);
  view.setUint16(34, bitsPerSample, true);
  writeString(view, 36, 'data');
  view.setUint32(40, dataSize, true);

  // Convert Float32 samples to Int16
  let maxSample = 0;
  for (let i = 0; i < audioData.length; i++) {
    const abs = Math.abs(audioData[i]);
    if (abs > maxSample) maxSample = abs;
  }

  // Normalize if needed to prevent clipping
  const normalizer = maxSample > 1 ? 1 / maxSample : 1;

  for (let i = 0; i < audioData.length; i++) {
    // Clamp to [-1, 1] and scale to Int16 range
    const s = Math.max(-1, Math.min(1, audioData[i] * normalizer));
    const sample = s < 0 ? s * 0x8000 : s * 0x7FFF;
    view.setInt16(headerSize + i * 2, sample, true);
  }

  return buffer;
}

function writeString(view: DataView, offset: number, str: string): void {
  for (let i = 0; i < str.length; i++) {
    view.setUint8(offset + i, str.charCodeAt(i));
  }
}

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = '';
  for (let i = 0; i < bytes.byteLength; i++) {
    binary += String.fromCharCode(bytes[i]);
  }
  return btoa(binary);
}
