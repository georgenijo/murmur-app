# Development Setup

## Prerequisites

- macOS 12+
- Node.js 18+
- Rust (install via rustup)
- Python 3.11+

## Setup

1. Clone the repository
2. Create Python virtual environment:
   ```bash
   python3 -m venv venv
   source venv/bin/activate
   pip install -r requirements.txt
   ```

3. Install Node dependencies:
   ```bash
   cd ui
   npm install
   ```

4. Run in development mode:
   ```bash
   cd ui
   npm run tauri dev
   ```

## macOS Permissions

Grant these permissions to your terminal app (e.g., Ghostty):
- System Settings > Privacy & Security > Microphone
- System Settings > Privacy & Security > Accessibility

## Building for Production

```bash
cd ui
npm run tauri build
```

Output: `ui/src-tauri/target/release/bundle/macos/local-dictation.app`
