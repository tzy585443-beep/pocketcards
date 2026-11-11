import json, re, subprocess, time
src = open("record-demo.js").read()
caps = [json.loads('"' + c + '"') for c in re.findall(r'await cap\("((?:[^"\\]|\\.)*)"', src)]
def speak(t):
    t = t.replace("...", ",").replace("0.15", "zero point one five").replace("0.01", "zero point zero one").replace("0.1 ", "zero point one ").replace("0.2 ", "zero point two ")
    t = t.replace("1 NEAR", "one NEAR").replace("NEP-171", "N E P one seventy one").replace("Code + tests: github.com/tzy585443-beep/pocketcards", "Code and tests are on GitHub.").replace("(testnet account)", "with a testnet account")
    return t
out = []
for i, c in enumerate(caps):
    f = f"vo{i}.mp3"
    for attempt in range(6):
        r = subprocess.run([".venv/bin/edge-tts", "--voice", "en-US-AndrewNeural", "--rate", "+6%", "--text", speak(c), "--write-media", f], capture_output=True)
        if r.returncode == 0: break
        time.sleep(3)
    else:
        raise SystemExit(f"tts failed for {i}")
    d = float(subprocess.check_output(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", f]).decode())
    out.append({"text": c, "file": f, "dur": d})
    print(i, round(d, 2), c[:50])
json.dump(out, open("tts.json", "w"), indent=1)
