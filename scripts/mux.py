import json, subprocess
marks = json.load(open("marks.json")); tts = json.load(open("tts.json"))
assert len(marks) == len(tts)
subprocess.run(["ffmpeg","-v","error","-y","-i","raw.webm","-c:v","libx264","-preset","medium","-crf","23","-pix_fmt","yuv420p","-r","24","base.mp4"], check=True)
inputs, filters = [], []
for i, (m, t) in enumerate(zip(marks, tts)):
    inputs += ["-i", t["file"]]
    ms = int(m["t"] * 1000) + 150
    filters.append(f"[{i+1}:a]adelay={ms}|{ms}[a{i}]")
mix = "".join(f"[a{i}]" for i in range(len(marks))) + f"amix=inputs={len(marks)}:normalize=0,apad[aout]"
subprocess.run(["ffmpeg","-v","error","-y","-i","base.mp4",*inputs,"-filter_complex",";".join(filters)+";"+mix,"-map","0:v","-map","[aout]","-c:v","copy","-c:a","aac","-b:a","128k","-ac","1","-shortest","-movflags","+faststart","pocketcards-demo.mp4"], check=True)
