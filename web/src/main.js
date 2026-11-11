import { Account, JsonRpcProvider, KeyPairSigner } from "near-api-js";
import { setupWalletSelector } from "@near-wallet-selector/core";
import { setupMyNearWallet } from "@near-wallet-selector/my-near-wallet";
import { setupModal } from "@near-wallet-selector/modal-ui";

const CONTRACT = new URLSearchParams(location.search).get("contract") || "pocketcards.testnet";
const RPC = "https://test.rpc.fastnear.com";
const EXPLORER = "https://testnet.nearblocks.io";
const YOCTO = 10n ** 24n;
const GAS = "50000000000000"; // 50 Tgas

const $ = (s) => document.querySelector(s);
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

// ---------- amounts ----------
export function toYocto(near) {
  const s = String(near).trim();
  if (!/^\d+(\.\d{0,24})?$/.test(s)) throw new Error(`"${near}" is not a NEAR amount`);
  const [i, f = ""] = s.split(".");
  return (BigInt(i) * YOCTO + BigInt((f + "0".repeat(24)).slice(0, 24))).toString();
}
export function fmt(yocto, dp = 2) {
  const v = BigInt(yocto);
  const whole = v / YOCTO;
  const frac = (v % YOCTO).toString().padStart(24, "0").slice(0, dp);
  return dp ? `${whole}.${frac}` : `${whole}`;
}

// ---------- RPC views ----------
async function view(method, args = {}) {
  const res = await fetch(RPC, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0", id: "pc", method: "query",
      params: { request_type: "call_function", finality: "optimistic", account_id: CONTRACT, method_name: method,
        args_base64: btoa(JSON.stringify(args)) },
    }),
  }).then((r) => r.json());
  if (res.error || res.result?.error) throw new Error(JSON.stringify(res.error || res.result.error));
  return JSON.parse(new TextDecoder().decode(new Uint8Array(res.result.result)));
}

// ---------- signing: wallet selector or testnet dev key ----------
let selector, modal, devAccount = null, accountId = null;

async function initWallet() {
  selector = await setupWalletSelector({ network: "testnet", modules: [setupMyNearWallet()] });
  modal = setupModal(selector, { contractId: CONTRACT });
  selector.store.observable.subscribe((s) => {
    const a = s.accounts.find((x) => x.active);
    if (a && !devAccount) setAccount(a.accountId);
  });
  const st = selector.store.getState();
  const a = st.accounts.find((x) => x.active);
  if (a) setAccount(a.accountId);
  const saved = sessionStorage.getItem("pc-devkey");
  if (!a && saved) {
    const { id, key } = JSON.parse(saved);
    useDevKey(id, key);
  }
}

function useDevKey(id, key) {
  const provider = new JsonRpcProvider({ url: RPC });
  devAccount = new Account(id, provider, KeyPairSigner.fromSecretKey(key));
  sessionStorage.setItem("pc-devkey", JSON.stringify({ id, key }));
  setAccount(id);
}

function setAccount(id) {
  accountId = id;
  $("#who").innerHTML = id
    ? `Signed in as <b>${esc(id)}</b> ${devAccount ? '<span class="tag">testnet dev key</span>' : '<span class="tag">wallet</span>'} <button id="signout" class="ghost">Sign out</button>`
    : "Not signed in";
  $("#signin").hidden = !!id;
  $("#app").classList.toggle("authed", !!id);
  if (id) {
    $("#signout").onclick = signOut;
    $("#mint-receiver").value ||= id;
    refreshMine(true);
  }
}

async function signOut() {
  if (devAccount) { devAccount = null; sessionStorage.removeItem("pc-devkey"); }
  else { const w = await selector.wallet(); await w.signOut(); }
  accountId = null;
  setAccount(null);
  $("#mine").innerHTML = "";
}

async function call(methodName, args, depositYocto = "0", label = methodName) {
  if (!accountId) throw new Error("Sign in first");
  const entry = log(`${label}…`, "pending");
  try {
    let outcome;
    if (devAccount) {
      outcome = await devAccount.callFunctionRaw({ contractId: CONTRACT, methodName, args, deposit: depositYocto, gas: GAS, waitUntil: "FINAL" });
    } else {
      const w = await selector.wallet();
      outcome = await w.signAndSendTransaction({
        receiverId: CONTRACT,
        actions: [{ type: "FunctionCall", params: { methodName, args, gas: GAS, deposit: depositYocto } }],
      });
    }
    const hash = outcome?.transaction?.hash || outcome?.transaction_outcome?.id;
    const failure = findFailure(outcome);
    const logs = (outcome?.receipts_outcome || []).flatMap((r) => r.outcome.logs);
    if (failure) throw new Error(failure);
    const restored = logs.some((l) => l.includes('"payout_failed"'));
    update(entry, `${label}: ${restored ? "transfer bounced, card balance restored" : "done"}`, restored ? "warn" : "ok", hash);
    return outcome;
  } catch (e) {
    update(entry, `${label}: ${cleanErr(e)}`, "err", e?.hash);
    try { e.__logged = true; } catch {}
    throw e;
  } finally {
    refreshAll(true);
  }
}

function findFailure(o) {
  if (!o) return null;
  const all = [o.transaction_outcome, ...(o.receipts_outcome || [])];
  for (const r of all) {
    const f = r?.outcome?.status?.Failure;
    if (f) return cleanErr(JSON.stringify(f));
  }
  return null;
}
function cleanErr(e) {
  const s = typeof e === "string" ? e : e?.message || JSON.stringify(e);
  const m = s.match(/(?:Smart contract|Contract) panicked: ([^"\\]+)/) || s.match(/panicked at[^:]*: ([^"\\]+)/);
  return m ? m[1] : s.slice(0, 220);
}

// ---------- activity log ----------
function log(msg, kind) {
  const li = document.createElement("li");
  li.className = kind;
  li.innerHTML = `<span>${esc(msg)}</span>`;
  $("#log").prepend(li);
  return li;
}
function update(li, msg, kind, hash) {
  li.className = kind;
  li.innerHTML = `<span>${esc(msg)}</span>` + (hash ? ` <a href="${EXPLORER}/txns/${hash}" target="_blank" rel="noopener">tx ↗</a>` : "");
}

// ---------- rendering ----------
function svgUri(v) {
  return "data:image/svg+xml;charset=utf-8," + encodeURIComponent(v.svg);
}
function cardHtml(v, actions) {
  const isOwner = v.owner_id === accountId, isIssuer = v.issuer === accountId;
  const expiry = v.expires_at_ms ? new Date(Number(v.expires_at_ms)).toLocaleString() : "never";
  const allow = v.allowlist ? v.allowlist.map(esc).join(", ") : "anyone";
  const resets = new Date(Number(v.window_resets_at_ms)).toLocaleTimeString();
  let act = "";
  if (actions && isOwner && !v.is_expired) {
    act += `<form data-act="spend" data-id="${v.token_id}"><b>Spend</b>
      <input name="to" placeholder="receiver.testnet" ${v.allowlist ? `list="al-${v.token_id}"` : ""} required>
      <input name="amt" placeholder="NEAR" size="6" required><button>Pay</button>
      ${v.allowlist ? `<datalist id="al-${v.token_id}">${v.allowlist.map((a) => `<option value="${esc(a)}">`).join("")}</datalist>` : ""}</form>
      <form data-act="transfer" data-id="${v.token_id}"><b>Give card</b>
      <input name="to" placeholder="new-holder.testnet" required><button>Transfer NFT</button></form>`;
  }
  if (actions && !v.is_expired) {
    act += `<form data-act="topup" data-id="${v.token_id}"><b>Top up</b><input name="amt" placeholder="NEAR" size="6" required><button>Add</button></form>`;
  }
  if (actions && isIssuer && v.is_expired && BigInt(v.balance) > 0n) {
    act += `<form data-act="reclaim" data-id="${v.token_id}"><button>Reclaim ${fmt(v.balance)} NEAR</button></form>`;
  }
  return `<article class="card" id="card-${v.token_id}">
    <img src="${svgUri(v)}" alt="PocketCard #${v.token_id}" width="320" height="200">
    <dl>
      <dt>Holder</dt><dd>${esc(v.owner_id)}${isOwner ? " (you)" : ""}</dd>
      <dt>Issuer</dt><dd>${esc(v.issuer)}${isIssuer ? " (you)" : ""}</dd>
      <dt>Balance</dt><dd><b>${fmt(v.balance, 4)} NEAR</b></dd>
      <dt>Spendable now</dt><dd>${fmt(v.available_now, 4)} NEAR${BigInt(v.daily_limit) > 0n ? ` <small>(cap ${fmt(v.daily_limit)}/24h, window resets ${resets})</small>` : ""}</dd>
      <dt>Pays</dt><dd>${allow}</dd>
      <dt>Expires</dt><dd>${expiry}${v.is_expired ? ' <span class="tag red">expired</span>' : ""}</dd>
      <dt>Spent</dt><dd>${fmt(v.total_spent, 4)} NEAR in ${v.spend_count} payment(s)${v.last_spend ? ` · last: ${fmt(v.last_spend.amount, 4)} → ${esc(v.last_spend.to)}` : ""}</dd>
    </dl>
    <div class="actions">${act}</div>
  </article>`;
}

// Re-render a card list only when its data changed and the user is not typing in it.
const lastHtml = {};
function paint(sel, html, force = false) {
  const el = $(sel);
  const busy = el.contains(document.activeElement) || [...el.querySelectorAll("input")].some((i) => i.value);
  if (!force && (busy || lastHtml[sel] === html)) return;
  lastHtml[sel] = html;
  el.innerHTML = html;
}

async function withSvg(cards) {
  return Promise.all(cards.map(async (c) => ({ ...c, svg: await view("card_svg", { token_id: c.token_id }) })));
}

async function refreshMine(force = false) {
  if (!accountId) return;
  const cards = await withSvg(await view("cards_for_owner", { account_id: accountId }));
  paint("#mine", cards.length ? cards.map((c) => cardHtml(c, true)).join("") : `<p class="muted">No cards held by ${esc(accountId)} yet. Mint one below, or ask someone to send you one.</p>`, force);
}
async function refreshRecent(force = false) {
  const cards = await withSvg(await view("recent_cards", { limit: 6 }));
  paint("#recent", cards.map((c) => cardHtml(c, !!accountId)).join("") || '<p class="muted">No cards yet.</p>', force);
  const supply = await view("nft_total_supply");
  $("#supply").textContent = `${supply} card(s) minted on ${CONTRACT}`;
}
async function refreshAll(force = false) {
  await Promise.allSettled([refreshMine(force), refreshRecent(force)]);
}

// ---------- events ----------
document.addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const f = ev.target;
  const d = Object.fromEntries(new FormData(f));
  try {
    switch (f.dataset.act || f.getAttribute("id")) {
      case "mint-form": {
        const allow = d.allowlist.split(/[\s,]+/).filter(Boolean);
        const exp = d.expires ? String(new Date(d.expires).getTime()) : null;
        await call("mint_card", {
          receiver_id: d.receiver.trim(), title: d.title, daily_limit: toYocto(d.limit || "0"),
          allowlist: allow.length ? allow : null, expires_at_ms: exp,
        }, toYocto(d.amount), `Mint "${d.title}" with ${d.amount} NEAR for ${d.receiver}`);
        break;
      }
      case "spend":
        await call("spend", { token_id: f.dataset.id, receiver_id: d.to.trim(), amount: toYocto(d.amt) }, "0", `Card #${f.dataset.id}: pay ${d.amt} NEAR to ${d.to}`);
        break;
      case "topup":
        await call("top_up", { token_id: f.dataset.id }, toYocto(d.amt), `Card #${f.dataset.id}: top up ${d.amt} NEAR`);
        break;
      case "transfer":
        await call("nft_transfer", { token_id: f.dataset.id, receiver_id: d.to.trim() }, "1", `Card #${f.dataset.id}: give to ${d.to}`);
        break;
      case "reclaim":
        await call("reclaim", { token_id: f.dataset.id }, "0", `Card #${f.dataset.id}: reclaim`);
        break;
      case "devkey-form":
        if (!/\.testnet$|^[0-9a-f]{64}$/.test(d.acct.trim())) throw new Error("dev keys are for testnet accounts only");
        useDevKey(d.acct.trim(), d.key.trim());
        f.reset();
        $("#devkey").open = false;
        break;
      case "lookup-form": {
        const cards = await withSvg(await view("cards_for_owner", { account_id: d.account.trim() }));
        $("#lookup").innerHTML = cards.map((c) => cardHtml(c, !!accountId)).join("") || `<p class="muted">${esc(d.account)} holds no cards.</p>`;
        break;
      }
    }
  } catch (e) {
    if (!e?.__logged) log(cleanErr(e), "err");
    console.error(e);
  }
});

$("#connect").onclick = () => modal.show();
$("#contract").innerHTML = `<a href="${EXPLORER}/address/${CONTRACT}" target="_blank" rel="noopener">${CONTRACT}</a>`;
refreshRecent().catch((e) => log(`RPC error: ${cleanErr(e)}`, "err"));
initWallet().catch((e) => log(`Wallet init failed: ${cleanErr(e)}`, "err"));
setInterval(() => refreshAll(false), 15000);
