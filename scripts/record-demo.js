const puppeteer = require("puppeteer-core");
const fs = require("fs");
const key = (id) => JSON.parse(fs.readFileSync(`${process.env.HOME}/.near-credentials/testnet/${id}.json`, "utf8")).private_key;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const URL = process.env.URL || "http://localhost:8765/";
const marks = [];
const KID = process.env.KID || "pc-maya.testnet";
let t0;

(async () => {
  const b = await puppeteer.launch({ executablePath: "/usr/bin/google-chrome", headless: "new", args: ["--no-sandbox", "--window-size=1280,800"] });
  const p = await b.newPage();
  await p.setViewport({ width: 1280, height: 800 });
  p.on("pageerror", (e) => console.log("pageerror:", e.message));
  await p.goto(URL, { waitUntil: "networkidle0" });
  await p.addStyleTag({ content: `#cap{position:fixed;left:0;right:0;bottom:0;z-index:99999;background:rgba(6,20,15,.88);color:#fff;font:600 22px/1.35 system-ui,sans-serif;padding:14px 28px;text-align:center}` });
  await p.evaluate(() => { const d = document.createElement("div"); d.id = "cap"; document.body.appendChild(d); });
  const rec = await p.screencast({ path: "raw.webm", fps: 24 });
  t0 = Date.now();
  const tts = JSON.parse(fs.readFileSync("tts.json", "utf8")); let ci = 0;
  const cap = async (text, hold = 2500) => { hold = Math.max(hold, Math.round(tts[ci++].dur * 1000) + 400); marks.push({ t: (Date.now() - t0) / 1000, text }); await p.$eval("#cap", (e, t) => (e.textContent = t), text); await sleep(hold); };
  const scrollTo = (sel) => p.$eval(sel, (e) => e.scrollIntoView({ behavior: "smooth", block: "center" })).then(() => sleep(900));
  const type = async (sel, text) => { await p.$eval(sel, (e) => (e.value = "")); await p.click(sel); await p.type(sel, text, { delay: 35 }); };
  const signIn = async (id) => {
    await scrollTo("#auth");
    await p.click("#devkey summary"); await sleep(400);
    await type("#devkey-form [name=acct]", id);
    await p.type("#devkey-form [name=key]", key(id), { delay: 2 });
    await p.click("#devkey-form button");
    await p.waitForFunction((id) => document.querySelector("#who").innerText.includes(id), {}, id);
    await sleep(1500);
  };
  let seen = 0;
  const waitLog = async () => {
    seen += 1;
    await p.waitForFunction((n) => { const lis = document.querySelectorAll("#log li"); return lis.length >= n && !lis[0].classList.contains("pending"); }, { timeout: 90000 }, seen);
    await sleep(2500);
    return p.$eval("#log li", (e) => e.className + " | " + e.innerText);
  };

  await cap("Giving someone money today means giving them all of it. PocketCards is an NFT that holds NEAR and spends it only by its rules.", 5000);
  await cap("The rules live in a NEAR smart contract on testnet: a daily cap, a payee allowlist and an optional expiry.", 4500);

  await cap("A parent signs in (testnet account) ...", 1200);
  await signIn("pc-parent.testnet");
  await cap("... and issues a card to their kid: 1 NEAR, max 0.2 NEAR per day, payable only to the school cafe.", 1500);
  await scrollTo("#mint-form");
  await type("#mint-form [name=receiver]", KID);
  await type("#mint-form [name=title]", "Lunch money");
  await type("#mint-form [name=amount]", "1");
  await type("#mint-form [name=limit]", "0.2");
  await type("#mint-form [name=allowlist]", "pc-cafe.testnet");
  await sleep(800);
  await p.click("#mint-form button");
  await scrollTo("#log");
  console.log("mint:", await waitLog());
  await cap("Minted. The NFT now holds the money; its card art is an on-chain SVG that shows the live balance.", 1000);
  await scrollTo("#recent");
  await sleep(3500);

  await p.click("#signout"); await sleep(800);
  await cap("Now the kid signs in. The card shows up under My cards.", 1200);
  await signIn(KID);
  await scrollTo("#mine");
  await sleep(2500);

  const spend = async (to, amt) => {
    const f = "#mine form[data-act=spend]";
    await scrollTo(f);
    await type(`${f} [name=to]`, to);
    await type(`${f} [name=amt]`, amt);
    await p.click(`${f} button`);
    await scrollTo("#log");
    return waitLog();
  };
  await cap("The kid pays the cafe 0.15 NEAR. The contract checks holder, allowlist, cap and balance, then sends the NEAR.", 1200);
  console.log("spend1:", await spend("pc-cafe.testnet", "0.15"));
  await cap("Paid. The card balance and today's spendable amount both drop.", 1000);
  await p.$eval("#mine", (e) => e.scrollIntoView({ behavior: "smooth", block: "start" })); await sleep(4000);

  await cap("Another 0.1 NEAR today would pass the 0.2 NEAR daily cap ...", 1200);
  console.log("spend2:", await spend("pc-cafe.testnet", "0.1"));
  await cap("... so the contract rejects it: \"daily limit exceeded for this card\".", 3500);

  await cap("Paying anyone who is not on the allowlist is rejected too.", 1200);
  console.log("spend3:", await spend("pc-parent.testnet", "0.01"));
  await cap("Rejected on-chain: \"receiver is not on this card's allowlist\".", 3500);

  await scrollTo("#mine");
  await cap("Because it's a standard NEP-171 NFT, any NEAR wallet can show it and pass it on. The money and the rules move together.", 5000);
  await cap("PocketCards: allowances, stipends, gift cards and capped budgets for AI agents. Code + tests: github.com/tzy585443-beep/pocketcards", 5500);
  await rec.stop();
  fs.writeFileSync("marks.json", JSON.stringify(marks, null, 1));
  await p.screenshot({ path: "final.png", fullPage: true });
  await b.close();
})().catch((e) => { console.error(e); process.exit(1); });
