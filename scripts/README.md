How the demo video was made (no camera, no face):

1. `python3 tts.py` turns each caption in `record-demo.js` into a narration clip (edge-tts) and writes `tts.json` with durations.
2. `node record-demo.js` drives the hosted app in headless Chrome with puppeteer against **testnet**, signs in with testnet dev keys read from `~/.near-credentials/testnet/` (never stored in this repo), shows captions, and records `raw.webm`. Every caption is held at least as long as its narration.
3. `python3 mux.py` encodes to H.264 and places each narration clip at its caption timestamp.

Env: `URL` (app URL, default http://localhost:8765/), `KID` (fresh testnet account that receives the card).
