import { DOT_ALPHAS, DOT_SCOPE, type DotScope, type ProcessingStyle } from './dotScope';

const COLORS = [[222 / 255, 215 / 255, 201 / 255], [200 / 255, 220 / 255, 248 / 255]];

// One point per dot; all circle coverage and the animated rounded clip run on
// the GPU. This avoids rebuilding/serializing hundreds of Canvas paths/frame.
const VERTEX = `
attribute vec4 dot;
uniform vec2 resolution;
uniform float ratio;
uniform float scale;
varying float radius;
varying float alpha;
varying float size;
void main() {
  vec2 position = dot.xy * scale * ratio;
  gl_Position = vec4(position * vec2(2.0, -2.0) / resolution, 0.0, 1.0);
  radius = dot.z * scale * ratio;
  size = ceil(2.0 * radius) + 2.0;
  gl_PointSize = size;
  alpha = dot.w;
}`;
const FRAGMENT = `
precision highp float;
uniform vec2 resolution;
uniform float ratio;
uniform float scale;
uniform vec3 clip;
uniform vec3 color;
varying float radius;
varying float alpha;
varying float size;
void main() {
  float circle = clamp(radius + 0.5 - length((gl_PointCoord - 0.5) * size), 0.0, 1.0);
  vec2 p = (gl_FragCoord.xy - resolution * 0.5) / (ratio * scale);
  vec2 q = abs(p) - clip.xy + clip.z;
  float distance = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - clip.z;
  float coverage = clamp(0.5 - distance * ratio * scale, 0.0, 1.0);
  float a = alpha * circle * coverage;
  gl_FragColor = vec4(color * a, a);
}`;

export interface GpuDotRenderer {
  paint(scope: DotScope, style: ProcessingStyle): void;
  dispose(): void;
}

/** Failed/blocked contexts and shader failures use the existing Canvas backend.
 * Resources are bounded per mounted effect and released on every exit path. */
export function createGpuDotRenderer(canvas: HTMLCanvasElement): GpuDotRenderer | null {
  let gl: WebGLRenderingContext | null = null;
  let vertex: WebGLShader | null = null;
  let fragment: WebGLShader | null = null;
  let program: WebGLProgram | null = null;
  let buffer: WebGLBuffer | null = null;
  const dispose = () => {
    if (!gl) return;
    if (buffer) gl.deleteBuffer(buffer);
    if (program) gl.deleteProgram(program);
    if (vertex) gl.deleteShader(vertex);
    if (fragment) gl.deleteShader(fragment);
  };
  try {
    gl = canvas.getContext('webgl', { alpha: true, antialias: false, depth: false,
      stencil: false, premultipliedAlpha: true, preserveDrawingBuffer: false,
      powerPreference: 'low-power' });
    if (!gl) return null;
    vertex = gl.createShader(gl.VERTEX_SHADER);
    fragment = gl.createShader(gl.FRAGMENT_SHADER);
    program = gl.createProgram(); buffer = gl.createBuffer();
    if (!vertex || !fragment || !program || !buffer) { dispose(); return null; }
    for (const [shader, source] of [[vertex, VERTEX], [fragment, FRAGMENT]] as const) {
      gl.shaderSource(shader, source); gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) { dispose(); return null; }
      gl.attachShader(program, shader);
    }
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) { dispose(); return null; }
    const dot = gl.getAttribLocation(program, 'dot');
    const uniforms = Object.fromEntries(['resolution', 'ratio', 'scale', 'clip', 'color']
      .map(name => [name, gl!.getUniformLocation(program!, name)]));
    if (dot < 0 || Object.values(uniforms).some(value => value === null)) { dispose(); return null; }
    const points = new Float32Array(DOT_SCOPE.columns * DOT_SCOPE.rows * 4);
    for (let n = 0; n < points.length / 4; n++) {
      points[n * 4] = (n % DOT_SCOPE.columns - 21) * DOT_SCOPE.pitch;
      points[n * 4 + 1] = (Math.floor(n / DOT_SCOPE.columns) - 4) * DOT_SCOPE.pitch;
    }
    gl.useProgram(program); gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, points.byteLength, gl.DYNAMIC_DRAW);
    gl.enableVertexAttribArray(dot); gl.vertexAttribPointer(dot, 4, gl.FLOAT, false, 0, 0);
    gl.enable(gl.BLEND); gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    gl.clearColor(0, 0, 0, 0);
    // Capture nonnullable references after successful initialization.
    const context = gl;
    return {
      dispose,
      paint(scope, style) {
        for (let n = 0; n < scope.dots.length; n++) {
          points[n * 4 + 2] = scope.radii[n];
          points[n * 4 + 3] = scope.buckets[n] / (DOT_ALPHAS - 1);
        }
        const ratio = canvas.width / scope.geometry.plateRecordingW;
        context.viewport(0, 0, canvas.width, canvas.height);
        context.clear(context.COLOR_BUFFER_BIT);
        context.uniform2f(uniforms.resolution, canvas.width, canvas.height);
        context.uniform1f(uniforms.ratio, ratio);
        context.uniform1f(uniforms.scale, scope.scale.x);
        context.uniform3f(uniforms.clip, scope.width.x / 2, scope.geometry.plateH / 2, scope.geometry.innerRadius);
        const color = COLORS[style === 'sweep' ? 1 : 0];
        context.uniform3f(uniforms.color, color[0], color[1], color[2]);
        context.bufferSubData(context.ARRAY_BUFFER, 0, points);
        context.drawArrays(context.POINTS, 0, scope.dots.length);
      },
    };
  } catch {
    // Some hosts throw rather than returning null when WebGL is disabled.
    // Cleanup must also tolerate a context that was lost during setup.
    try { dispose(); } catch { /* no live resources remain usable */ }
    return null;
  }
}
