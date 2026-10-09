#!/usr/bin/env node
// QUBIT launchpad API. Listens on 127.0.0.1 only; nginx publishes it.
//
// Every token's mint address comes from one slice of IBM quantum output (see
// RULE.md). Unused slices are future private keys: they never leave this
// process, and only launched slices are revealed through /token/<mint>. Only
// sealed jobs are served: their slices are fixed by a published SHA-256 Merkle
// root, and every token's proof carries its slice's path to that root.
//
//   POST /launch/prepare   reserve a ready mint, build create_v2 (+ dev buy), sign it with the mint key; rewards
//                          "creator" (fees to the launcher's wallet), "holders", or "vault" (the launcher's QUBIT
//                          Vault treasury is the pump creator)
//   POST /launch/confirm   record a launch after checking it on-chain
//   POST /fees/collect     unsigned transaction that collects a vault token's creator fees into its treasury
//   GET  /token/<mint>     proof of a launched token, its creator and its top holders
//   GET  /tokens           launched tokens of this pool, newest first (?cursor=&limit=)
//   GET  /stats            pool stock and sealed roots

"use strict";

const crypto = require("node:crypto");
const http = require("node:http");
const path = require("node:path");
const { DatabaseSync } = require("node:sqlite");
const {
  ComputeBudgetProgram, Connection, Keypair, LAMPORTS_PER_SOL, PublicKey,
  TransactionInstruction, TransactionMessage, VersionedTransaction,
} = require("@solana/web3.js");
const { NATIVE_MINT, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID, getAssociatedTokenAddressSync } = require("@solana/spl-token");
const BN = require("bn.js");
const {
  OnlinePumpSdk, PUMP_SDK, ammCreatorVaultPda, bondingCurvePda, canonicalPumpPoolPda, creatorVaultPda, getBuyTokenAmountFromSolAmount,
} = require("@pump-fun/pump-sdk");

const PORT = Number(process.env.PORT || 8787);
const DB_PATH = process.env.POOL_DB || path.join(__dirname, "..", "private", "launchpad", "pool.db");
const RPC_URL = process.env.RPC_URL || "https://solana-rpc.publicnode.com";
// RPC that serves indexed token queries (largest accounts, supply, holder scan). Its URL carries an API key: it is never
// logged or returned. Without it, token pages show no holder data.
const INDEX_RPC_URL = process.env.INDEX_RPC_URL || "";
const HOLDERS_TTL_MS = 30_000;
const SERVE_DRY = process.env.SERVE_DRY === "1";
// Exact origins, or patterns where "*" stands for one run of [a-z0-9-] (e.g. https://qubit-*-team.vercel.app).
const ALLOWED_ORIGINS = (process.env.ALLOWED_ORIGINS || "https://bornfromqubit.com").split(",").map((origin) => origin.trim()).filter(Boolean)
  .map((origin) => (origin.includes("*")
    ? new RegExp(`^${origin.split("*").map((part) => part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("[a-z0-9-]+")}$`)
    : origin));
const MICRO_LAMPORTS = Number(process.env.CU_PRICE || 1_000_000);
const SUFFIX = "qbit";
const RULE_URL = "github.com/humdonsolana/QUBIT/tree/main/launchpad";
const MEMO_PROGRAM_ID = new PublicKey("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
// QUBIT Vault program (vault/crates/interface/src/lib.rs): an 80-byte vault account, and a treasury PDA that holds its funds.
const VAULT_PROGRAM_ID = new PublicKey("3kQoxmBhrQztTBRVhqkGbMbUcpQpK64cbadrt2y9kbjX");
const VAULT_LEN = 80;
const TREASURY_SEED = Buffer.from("treasury");
// PumpSwap pool account: coin_creator pubkey at byte 211, and at byte 279 the u64 creator-fee bucket that v3 trades fill
// (newer than the pool decoder in @pump-fun/pump-swap-sdk, so it is read directly).
const POOL_COIN_CREATOR = 211;
const POOL_CREATOR_FEE = 279;
// pump.fun's own lookup table; without it create_v2 + buy does not fit in one transaction.
const PUMP_LOOKUP_TABLE = new PublicKey("Hyif6eWb8x88RVrvjPfabsgRYnwkVnyByEXTVTXbUcyP");
const RESERVATION_MS = 10 * 60_000;
const MAX_BODY_BYTES = 6 * 1024 * 1024;
const MAX_IMAGE_BYTES = 4 * 1024 * 1024;
const MAX_DEV_BUY_SOL = 50;
const PREPARES_PER_HOUR = 10;
const IMAGE_TYPES = { "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif", "image/webp": "webp" };

const db = new DatabaseSync(DB_PATH);
db.exec("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;");
const connection = new Connection(RPC_URL, "confirmed");
const online = new OnlinePumpSdk(connection);
const prepares = new Map();
const collects = new Map();
const trees = new Map();
const treasuries = new Map();
const holderCache = new Map();

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

const sha256 = (data) => crypto.createHash("sha256").update(data).digest();
const nowIso = () => new Date().toISOString();
const originAllowed = (origin) => ALLOWED_ORIGINS.some((allowed) => (typeof allowed === "string" ? allowed === origin : allowed.test(origin)));

// The treasury PDA of a vault address, derived the way the vault program does when it creates the vault.
function treasuryOf(vault) {
  if (!treasuries.has(vault)) {
    treasuries.set(vault, PublicKey.findProgramAddressSync([TREASURY_SEED, new PublicKey(vault).toBuffer()], VAULT_PROGRAM_ID)[0]);
  }
  return treasuries.get(vault);
}

// A QUBIT Vault account (owner, length, discriminator and version checked) and the treasury it controls.
async function loadVault(address) {
  const refusal = new HttpError(400, "vault must be a QUBIT Vault address (the vault: line of `qubit address`)");
  let vault;
  try {
    vault = new PublicKey(address);
  } catch {
    throw refusal;
  }
  const account = await connection.getAccountInfo(vault, "confirmed");
  if (!account?.owner.equals(VAULT_PROGRAM_ID) || account.data.length !== VAULT_LEN || account.data[0] !== 1 || account.data[1] !== 1) {
    throw refusal;
  }
  try {
    const treasury = PublicKey.createProgramAddressSync([TREASURY_SEED, vault.toBuffer(), Buffer.from([account.data[3]])], VAULT_PROGRAM_ID);
    return { vault, treasury };
  } catch {
    throw refusal;
  }
}

function entropyOf(slot) {
  const bits = JSON.parse(slot.bitstrings).join("");
  if (bits.length < 256 || /[^01]/.test(bits)) throw new Error(`slot ${slot.id} has malformed bits`);
  return Buffer.from(BigInt("0b" + bits.slice(0, 256)).toString(16).padStart(64, "0"), "hex");
}

function slotSecrets(slot) {
  const entropy = entropyOf(slot);
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(slot.counter));
  const keypair = Keypair.fromSeed(sha256(Buffer.concat([entropy, counter])));
  if (keypair.publicKey.toBase58() !== slot.mint || !slot.mint.endsWith(SUFFIX)) {
    throw new Error(`slot ${slot.id} does not derive its stored mint`);
  }
  return keypair;
}

// Merkle tree over a job's slices, rebuilt from the bits and checked against the root sealed by pool.py.
function poolTree(jobId) {
  if (trees.has(jobId)) return trees.get(jobId);
  const job = db.prepare("SELECT merkle_root FROM jobs WHERE job_id = ?").get(jobId);
  const slots = db.prepare("SELECT id, slice_index, bitstrings FROM slots WHERE job_id = ? ORDER BY slice_index").all(jobId);
  let level = slots.map((slot, k) => {
    if (slot.slice_index !== k) throw new Error(`job ${jobId} is missing slice ${k}`);
    return sha256(Buffer.concat([Buffer.from([0]), entropyOf(slot)]));
  });
  const slices = level.length;
  while (level.length & (level.length - 1)) level.push(Buffer.alloc(32));
  const levels = [level];
  while (level.length > 1) {
    const next = [];
    for (let i = 0; i < level.length; i += 2) next.push(sha256(Buffer.concat([Buffer.from([1]), level[i], level[i + 1]])));
    levels.push(level = next);
  }
  const root = level[0].toString("hex");
  if (!job?.merkle_root || root !== job.merkle_root) throw new Error(`job ${jobId} does not match its sealed root`);
  const tree = { root, slices, levels };
  trees.set(jobId, tree);
  return tree;
}

const merklePath = (tree, index) => tree.levels.slice(0, -1).map((level, i) => level[(index >> i) ^ 1].toString("hex"));

function proofMemo(slot, job, root, withRule) {
  const tag = job.dry_run
    ? "QUBIT launchpad DRY RUN, local simulator, not quantum hardware."
    : "QUBIT launchpad: mint derived from IBM quantum computer output.";
  const facts = `job ${slot.job_id} ${job.backend}, slice ${slot.slice_index}, counter ${slot.counter}, sealed pool root ${root}`;
  return `${tag} ${facts}${withRule ? `, rule ${RULE_URL}` : ""}`;
}

function text(value, field, max, required = true) {
  const out = typeof value === "string" ? value.trim() : "";
  if (required && !out) throw new HttpError(400, `${field} is required`);
  if (out.length > max) throw new HttpError(400, `${field} is longer than ${max} characters`);
  return out;
}

function link(value, field) {
  const out = text(value, field, 200, false);
  if (out && !/^https:\/\/\S+$/.test(out)) throw new HttpError(400, `${field} must be an https:// link`);
  return out;
}

function parseLaunch(body) {
  let creator;
  try {
    creator = new PublicKey(text(body.creator, "creator", 64));
  } catch {
    throw new HttpError(400, "creator must be a Solana address");
  }
  const devBuySol = Number(body.devBuySol ?? 0);
  if (!Number.isFinite(devBuySol) || devBuySol < 0 || devBuySol > MAX_DEV_BUY_SOL) {
    throw new HttpError(400, `devBuySol must be between 0 and ${MAX_DEV_BUY_SOL}`);
  }
  if (!["creator", "holders", "vault"].includes(body.rewards)) throw new HttpError(400, 'rewards must be "creator", "holders" or "vault"');
  const params = {
    creator,
    name: text(body.name, "name", 32),
    symbol: text(body.symbol, "symbol", 10),
    description: text(body.description, "description", 500, false),
    twitter: link(body.twitter, "twitter"),
    telegram: link(body.telegram, "telegram"),
    website: link(body.website, "website"),
    devBuyLamports: Math.round(devBuySol * LAMPORTS_PER_SOL),
    rewards: body.rewards,
    vault: body.rewards === "vault" ? text(body.vault, "vault", 64) : null,
  };
  if (SERVE_DRY && typeof body.metadataUri === "string") return { ...params, metadataUri: body.metadataUri };
  const match = /^data:(image\/[a-z]+);base64,([A-Za-z0-9+/=]+)$/.exec(body.image || "");
  if (!match || !IMAGE_TYPES[match[1]]) throw new HttpError(400, "image must be a png, jpeg, gif or webp data URL");
  const image = Buffer.from(match[2], "base64");
  if (image.length > MAX_IMAGE_BYTES) throw new HttpError(400, "image is larger than 4 MB");
  return { ...params, image, imageType: match[1] };
}

async function uploadMetadata(p) {
  const form = new FormData();
  form.append("file", new Blob([p.image], { type: p.imageType }), `image.${IMAGE_TYPES[p.imageType]}`);
  for (const key of ["name", "symbol", "description", "twitter", "telegram", "website"]) form.append(key, p[key]);
  form.append("showName", "true");
  const res = await fetch("https://pump.fun/api/ipfs", { method: "POST", body: form });
  const body = await res.text();
  if (!res.ok) throw new HttpError(502, `metadata upload failed: HTTP ${res.status}`);
  const { metadataUri, metadata } = JSON.parse(body);
  if (!metadataUri) throw new HttpError(502, "metadata upload returned no URI");
  return { metadataUri, image: typeof metadata?.image === "string" ? metadata.image : null };
}

function reserveSlot(creator) {
  return db.prepare(
    "UPDATE slots SET status = 'reserved', reserved_at = ?, reserved_for = ? WHERE id = (" +
    "SELECT slots.id FROM slots JOIN jobs USING (job_id) WHERE slots.status = 'ready' AND jobs.dry_run = ? " +
    "AND jobs.merkle_root IS NOT NULL ORDER BY slots.id LIMIT 1) RETURNING *",
  ).get(nowIso(), creator, SERVE_DRY ? 1 : 0);
}

// treasury: the vault treasury that becomes the pump creator ("vault" rewards); otherwise the launcher wallet is the creator.
async function buildTransaction({ p, uri, mintKeypair, memo, computeUnits, treasury }) {
  const [global, feeConfig, { value: table }] = await Promise.all([
    online.fetchGlobal(), online.fetchFeeConfig(), connection.getAddressLookupTable(PUMP_LOOKUP_TABLE),
  ]);
  if (!table?.isActive()) throw new HttpError(503, "pump.fun lookup table is unavailable");
  const base = {
    global, mint: mintKeypair.publicKey, name: p.name, symbol: p.symbol, uri,
    creator: treasury || p.creator, user: p.creator, mayhemMode: false, holderReward: p.rewards === "holders",
  };
  let instructions;
  try {
    if (p.devBuyLamports > 0) {
      const solAmount = new BN(p.devBuyLamports);
      const amount = getBuyTokenAmountFromSolAmount({
        global, feeConfig, mintSupply: null, bondingCurve: null, amount: solAmount, quoteMint: NATIVE_MINT,
      });
      instructions = await PUMP_SDK.createV2AndBuyInstructions({ ...base, amount, solAmount });
    } else {
      instructions = [await PUMP_SDK.createV2Instruction(base)];
    }
  } catch (error) {
    throw new HttpError(400, `pump.fun refused these settings: ${error.message}`);
  }
  const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash("confirmed");
  const message = new TransactionMessage({
    payerKey: p.creator,
    recentBlockhash: blockhash,
    instructions: [
      ComputeBudgetProgram.setComputeUnitLimit({ units: computeUnits }),
      ComputeBudgetProgram.setComputeUnitPrice({ microLamports: MICRO_LAMPORTS }),
      ...instructions,
      new TransactionInstruction({ programId: MEMO_PROGRAM_ID, keys: [], data: Buffer.from(memo, "utf8") }),
    ],
  }).compileToV0Message([table]);
  return { tx: new VersionedTransaction(message), lastValidBlockHeight };
}

// action: what the wallet is asked to do, for the error messages ("launch" or "fee collection").
async function simulate(tx, action = "launch") {
  const { value } = await connection.simulateTransaction(tx, { sigVerify: false, replaceRecentBlockhash: true });
  if (value.err) {
    const logs = (value.logs || []).join("\n");
    if (/insufficient lamports|insufficient funds|no record of a prior credit/i.test(logs + JSON.stringify(value.err))) {
      throw new HttpError(400, `the ${action === "launch" ? "launcher" : "payer"} wallet does not have enough SOL for this ${action}`);
    }
    throw new HttpError(400, `${action} would fail: ${(value.logs || []).slice(-1)[0] || JSON.stringify(value.err)}`);
  }
  return value.unitsConsumed;
}

// At most PREPARES_PER_HOUR calls per address per hour in `store`.
function throttle(store, ip, what) {
  const hits = (store.get(ip) || []).filter((t) => Date.now() - t < 3_600_000);
  if (hits.length >= PREPARES_PER_HOUR) throw new HttpError(429, `too many ${what}, try again later`);
  store.set(ip, [...hits, Date.now()]);
}

async function prepare(body, ip) {
  throttle(prepares, ip, "launch attempts");

  const p = parseLaunch(body);
  // Checked before a slot is reserved, so a bad vault leaves the pool untouched.
  const vault = p.rewards === "vault" ? await loadVault(p.vault) : null;
  const slot = reserveSlot(p.creator.toBase58());
  if (!slot) throw new HttpError(503, "no quantum mint is ready right now, try again in a few minutes");
  let handedOut = false;
  try {
    const job = db.prepare("SELECT * FROM jobs WHERE job_id = ?").get(slot.job_id);
    const mintKeypair = slotSecrets(slot);
    const { root } = poolTree(slot.job_id);
    const { metadataUri: uri, image } = p.metadataUri ? { metadataUri: p.metadataUri, image: null } : await uploadMetadata(p);
    const treasury = vault?.treasury;
    let memo = proofMemo(slot, job, root, true);
    let probe = await buildTransaction({ p, uri, mintKeypair, memo, computeUnits: 1_400_000, treasury });
    if (probe.tx.serialize().length > 1232) {
      memo = proofMemo(slot, job, root, false);
      probe = await buildTransaction({ p, uri, mintKeypair, memo, computeUnits: 1_400_000, treasury });
      if (probe.tx.serialize().length > 1232) throw new HttpError(400, "name, symbol and metadata link are too long for one transaction");
    }
    const units = await simulate(probe.tx);
    const built = await buildTransaction({ p, uri, mintKeypair, memo, computeUnits: Math.ceil(units * 1.25), treasury });
    built.tx.sign([mintKeypair]);
    db.prepare("UPDATE slots SET name = ?, symbol = ?, rewards = ?, vault = ?, uri = ?, image = ? WHERE id = ?")
      .run(p.name, p.symbol, p.rewards, vault ? vault.vault.toBase58() : null, uri, image, slot.id);
    handedOut = true;
    console.log(`prepared ${slot.mint} for ${p.creator.toBase58()} (slot ${slot.id}, ${p.rewards} rewards)`);
    return {
      mint: slot.mint,
      transaction: Buffer.from(built.tx.serialize()).toString("base64"),
      lastValidBlockHeight: built.lastValidBlockHeight,
      expiresAt: new Date(Date.parse(slot.reserved_at) + RESERVATION_MS).toISOString(),
      rewards: p.rewards,
      proof: { job: slot.job_id, slice: slot.slice_index, root, dryRun: Boolean(job.dry_run) },
    };
  } finally {
    // Nothing left the server: the mint is still unknown to anyone, so it can be offered again.
    if (!handedOut) db.prepare("UPDATE slots SET status = 'ready', reserved_at = NULL, reserved_for = NULL WHERE id = ?").run(slot.id);
  }
}

async function confirm(body) {
  let mint;
  try {
    mint = new PublicKey(String(body.mint)).toBase58();
  } catch {
    throw new HttpError(400, "mint must be a Solana address");
  }
  const slot = db.prepare("SELECT * FROM slots WHERE mint = ?").get(mint);
  if (!slot) throw new HttpError(404, "unknown mint");
  if (slot.status === "launched") return proof(slot);
  if (slot.status !== "reserved") throw new HttpError(409, `mint is ${slot.status}`);
  const signature = text(body.signature, "signature", 120);
  const tx = await connection.getTransaction(signature, { maxSupportedTransactionVersion: 0, commitment: "confirmed" });
  if (!tx) throw new HttpError(404, "transaction not found yet, try again in a few seconds");
  if (tx.meta?.err) throw new HttpError(400, "the launch transaction failed on-chain");
  const keys = tx.transaction.message.staticAccountKeys.map(String);
  const signers = keys.slice(0, tx.transaction.message.header.numRequiredSignatures);
  const logs = (tx.meta?.logMessages || []).join("\n");
  if (!signers.includes(mint) || !logs.includes(`slice ${slot.slice_index}, counter ${slot.counter}`)) {
    throw new HttpError(400, "transaction is not the launch of this mint");
  }
  const account = await connection.getAccountInfo(new PublicKey(mint), "confirmed");
  if (!account?.owner.equals(TOKEN_2022_PROGRAM_ID)) throw new HttpError(400, "mint does not exist on-chain");
  markLaunched(slot.id, signature, new Date(tx.blockTime * 1000).toISOString());
  return proof(db.prepare("SELECT * FROM slots WHERE id = ?").get(slot.id));
}

function markLaunched(id, signature, launchedAt) {
  db.prepare("UPDATE slots SET status = 'launched', launch_sig = ?, launched_at = ? WHERE id = ?").run(signature, launchedAt, id);
  console.log(`launched slot ${id} in ${signature}`);
}

function proof(slot) {
  const job = db.prepare("SELECT * FROM jobs WHERE job_id = ?").get(slot.job_id);
  slotSecrets(slot); // throws unless the bits still derive the stored mint
  const tree = poolTree(slot.job_id);
  const vault = slot.rewards === "vault" ? slot.vault : null;
  return {
    mint: slot.mint, name: slot.name, symbol: slot.symbol,
    launchSignature: slot.launch_sig, launchedAt: slot.launched_at,
    creator: {
      rewards: slot.rewards ?? null, wallet: slot.reserved_for, vault, treasury: vault ? treasuryOf(vault).toBase58() : null,
    },
    job: { id: job.job_id, backend: job.backend, shots: job.shots, dryRun: Boolean(job.dry_run) },
    slice: slot.slice_index,
    bitstrings: JSON.parse(slot.bitstrings),
    counter: slot.counter,
    pool: {
      root: tree.root, slices: tree.slices, sealedAt: job.sealed_utc, path: merklePath(tree, slot.slice_index),
      rule: "leaf = SHA-256(0x00 || entropy), node = SHA-256(0x01 || left || right), leaves padded with 32 zero " +
        "bytes to a power of two; the path runs from leaf to root, and bit i of the slice number set means sibling i is on the left",
    },
    rule: `seed = SHA-256(first 256 bits of the 16 bitstrings, MSB-first || n as 8-byte big-endian); ` +
      `mint = base58(ed25519 public key); n is the smallest counter whose mint ends in "${SUFFIX}"`,
  };
}

async function token(mint) {
  const slot = db.prepare("SELECT * FROM slots WHERE mint = ? AND status = 'launched'").get(mint);
  if (!slot) throw new HttpError(404, "no launched token with this mint");
  return { ...proof(slot), ...await holders(mint) };
}

// Top ten holders ({owner, percent, curve}) and the number of accounts holding any of the token, cached HOLDERS_TTL_MS.
// `curve` marks the pump.fun bonding curve or its PumpSwap pool, which hold the unsold supply rather than a person.
async function holders(mint) {
  if (!INDEX_RPC_URL) return { holders: null, holderCount: null };
  const hit = holderCache.get(mint);
  if (hit && Date.now() - hit.at < HOLDERS_TTL_MS) return hit.value;
  try {
    const key = new PublicKey(mint);
    const [largest, supply, accounts] = await Promise.all([
      rpc("getTokenLargestAccounts", [mint, { commitment: "confirmed" }], INDEX_RPC_URL),
      rpc("getTokenSupply", [mint, { commitment: "confirmed" }], INDEX_RPC_URL),
      rpc("getProgramAccounts", [TOKEN_2022_PROGRAM_ID.toBase58(), {
        encoding: "base64", commitment: "confirmed", dataSlice: { offset: 64, length: 8 }, filters: [{ memcmp: { offset: 0, bytes: mint } }],
      }], INDEX_RPC_URL),
    ]);
    const total = BigInt(supply.value.amount);
    const top = largest.value.filter((account) => BigInt(account.amount) > 0n);
    const owners = top.length
      ? (await rpc("getMultipleAccounts", [top.map((account) => account.address), { encoding: "jsonParsed", commitment: "confirmed" }], INDEX_RPC_URL)).value
      : [];
    const market = new Set([bondingCurvePda(key).toBase58(), canonicalPumpPoolPda(key).toBase58()]);
    const value = {
      holders: top.map((account, i) => {
        const owner = owners[i]?.data?.parsed?.info?.owner;
        return owner && { owner, percent: total ? Number(BigInt(account.amount) * 1_000_000n / total) / 10_000 : 0, curve: market.has(owner) };
      }).filter(Boolean).slice(0, 10),
      holderCount: accounts.filter((account) => Buffer.from(account.account.data[0], "base64").readBigUInt64LE(0) > 0n).length,
    };
    holderCache.set(mint, { at: Date.now(), value });
    return value;
  } catch (error) {
    console.error(`holders of ${mint}: ${error.message}`);
    const value = { holders: null, holderCount: null };
    holderCache.set(mint, { at: Date.now(), value });
    return value;
  }
}

async function rpc(method, params, url = RPC_URL) {
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json", "User-Agent": "Mozilla/5.0" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  const body = await res.json();
  if (body.error) throw new Error(`${method}: ${body.error.message}`);
  return body.result;
}

// Launched tokens of this pool, newest first. `cursor` is the launchedAt that ended the previous page (strictly older
// tokens follow). Equal timestamps cannot be told apart by such a cursor, so a page never ends inside a tie: it may run
// past `limit` by the tokens sharing its last timestamp.
function tokens(params) {
  const asked = Number(params.get("limit") ?? 50);
  const limit = Number.isFinite(asked) ? Math.min(Math.max(Math.trunc(asked), 1), 100) : 50;
  const cursor = params.get("cursor");
  if (cursor !== null && Number.isNaN(Date.parse(cursor))) throw new HttpError(400, "cursor must be the launchedAt of the last token of the previous page");
  const dry = SERVE_DRY ? 1 : 0;
  const select = "SELECT slots.*, jobs.backend AS backend, jobs.shots AS shots, jobs.dry_run AS dry_run FROM slots " +
    "JOIN jobs USING (job_id) WHERE slots.status = 'launched' AND jobs.dry_run = ?";
  const order = " ORDER BY slots.launched_at DESC, slots.id DESC";
  const rows = cursor === null
    ? db.prepare(`${select}${order} LIMIT ?`).all(dry, limit + 1)
    : db.prepare(`${select} AND slots.launched_at < ?${order} LIMIT ?`).all(dry, cursor, limit + 1);
  const page = rows.slice(0, limit);
  let next = null;
  if (rows.length > limit) {
    const last = page[page.length - 1];
    page.push(...db.prepare(`${select} AND slots.launched_at = ? AND slots.id < ? ORDER BY slots.id DESC`).all(dry, last.launched_at, last.id));
    if (db.prepare(`${select} AND slots.launched_at < ? LIMIT 1`).get(dry, last.launched_at)) next = last.launched_at;
  }
  return {
    tokens: page.map((r) => ({
      mint: r.mint, name: r.name, symbol: r.symbol, image: r.image ?? null, uri: r.uri ?? null,
      launchedAt: r.launched_at, launchSignature: r.launch_sig, creator: r.reserved_for,
      rewards: r.rewards ?? null, vault: r.vault ?? null, slice: r.slice_index,
      job: { id: r.job_id, backend: r.backend, shots: r.shots, dryRun: Boolean(r.dry_run) },
    })),
    next,
  };
}

// Everything a vault treasury is owed on `mint`, as instructions only the payer signs (nobody signs for the creator):
// first the fee buckets that v3 trades leave on the curve and in the PumpSwap pool are swept into the creator vaults,
// then the SDK's collect pays the pump creator vault to the treasury in SOL and the PumpSwap vault into its WSOL account.
async function collectInstructions(mint, creator, payer) {
  const ammVault = getAssociatedTokenAddressSync(NATIVE_MINT, ammCreatorVaultPda(creator), true, TOKEN_PROGRAM_ID);
  const [curveInfo, poolInfo, vaultInfo, ammVaultInfo] = await connection.getMultipleAccountsInfo(
    [bondingCurvePda(mint), canonicalPumpPoolPda(mint), creatorVaultPda(creator), ammVault], "confirmed",
  );
  const sweeps = [];
  let pending = 0n;
  if (curveInfo) {
    const curve = PUMP_SDK.decodeBondingCurve(curveInfo);
    if (!curve.creator.equals(creator)) throw new HttpError(409, "the creator of this token on pump.fun is not its vault treasury");
    if (!curve.quoteMint.equals(PublicKey.default)) throw new HttpError(409, "this token is not quoted in SOL");
    const bucket = BigInt(curve.creatorFee.toString());
    if (bucket > 0n) {
      sweeps.push(await PUMP_SDK.sweepCreatorFeeInstruction({ payer, mint, creator, quoteMint: curve.quoteMint }));
      pending += bucket;
    }
  }
  if (poolInfo && poolInfo.data.length >= POOL_CREATOR_FEE + 8 &&
      new PublicKey(poolInfo.data.subarray(POOL_COIN_CREATOR, POOL_COIN_CREATOR + 32)).equals(creator)) {
    const bucket = poolInfo.data.readBigUInt64LE(POOL_CREATOR_FEE);
    if (bucket > 0n) {
      sweeps.push(await PUMP_SDK.sweepPoolCreatorFeeInstruction({ payer, mint, coinCreator: creator, quoteMint: NATIVE_MINT }));
      pending += bucket;
    }
  }
  if (vaultInfo) pending += BigInt(Math.max(0, vaultInfo.lamports - await connection.getMinimumBalanceForRentExemption(vaultInfo.data.length)));
  if (ammVaultInfo?.owner.equals(TOKEN_PROGRAM_ID) && ammVaultInfo.data.length >= 72) pending += ammVaultInfo.data.readBigUInt64LE(64);
  if (pending === 0n) throw new HttpError(409, "no creator fees to collect yet");
  return [...sweeps, ...await online.collectCoinCreatorFeeInstructions(creator, payer)];
}

async function collectFees(body, ip) {
  throttle(collects, ip, "fee collections");
  let mint, payer;
  try {
    mint = new PublicKey(String(body.mint));
  } catch {
    throw new HttpError(400, "mint must be a Solana address");
  }
  try {
    payer = new PublicKey(String(body.payer));
  } catch {
    throw new HttpError(400, "payer must be a Solana address");
  }
  const slot = db.prepare(
    "SELECT slots.* FROM slots JOIN jobs USING (job_id) WHERE slots.mint = ? AND slots.status = 'launched' AND jobs.dry_run = ?",
  ).get(mint.toBase58(), SERVE_DRY ? 1 : 0);
  if (slot?.rewards !== "vault") throw new HttpError(400, "mint is not a vault token launched here");
  const instructions = await collectInstructions(mint, treasuryOf(slot.vault), payer);
  const build = async (units) => {
    const { blockhash, lastValidBlockHeight } = await connection.getLatestBlockhash("confirmed");
    const message = new TransactionMessage({
      payerKey: payer,
      recentBlockhash: blockhash,
      instructions: [
        ComputeBudgetProgram.setComputeUnitLimit({ units }),
        ComputeBudgetProgram.setComputeUnitPrice({ microLamports: MICRO_LAMPORTS }),
        ...instructions,
      ],
    }).compileToV0Message();
    return { tx: new VersionedTransaction(message), lastValidBlockHeight };
  };
  const units = await simulate((await build(1_400_000)).tx, "fee collection");
  const { tx, lastValidBlockHeight } = await build(Math.ceil(units * 1.25));
  if (tx.serialize().length > 1232) throw new HttpError(400, "the fee collection does not fit one transaction");
  console.log(`fee collection for ${mint.toBase58()} by ${payer.toBase58()}`);
  return { transaction: Buffer.from(tx.serialize()).toString("base64"), lastValidBlockHeight };
}

function stats() {
  const rows = db.prepare(
    "SELECT slots.status AS status, COUNT(*) AS count FROM slots JOIN jobs USING (job_id) " +
    "WHERE jobs.dry_run = ? GROUP BY slots.status",
  ).all(SERVE_DRY ? 1 : 0);
  const seals = db.prepare(
    "SELECT job_id AS job, backend, shots, merkle_root AS root, sealed_utc AS sealedAt FROM jobs " +
    "WHERE dry_run = ? AND merkle_root IS NOT NULL ORDER BY sealed_utc",
  ).all(SERVE_DRY ? 1 : 0);
  return { pool: SERVE_DRY ? "dry-run" : "ibm", ...Object.fromEntries(rows.map((r) => [r.status, r.count])), seals };
}

async function expireReservations() {
  const stale = db.prepare("SELECT * FROM slots WHERE status = 'reserved' AND reserved_at < ?")
    .all(new Date(Date.now() - RESERVATION_MS).toISOString());
  for (const slot of stale) {
    const mint = new PublicKey(slot.mint);
    if (await connection.getAccountInfo(mint, "confirmed")) {
      const sigs = await connection.getSignaturesForAddress(mint, { limit: 1000 });
      const first = sigs[sigs.length - 1];
      markLaunched(slot.id, first?.signature || null, first?.blockTime ? new Date(first.blockTime * 1000).toISOString() : nowIso());
    } else {
      db.prepare("UPDATE slots SET status = 'retired' WHERE id = ? AND status = 'reserved'").run(slot.id);
      console.log(`retired unused reservation ${slot.id}`);
    }
  }
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on("data", (chunk) => {
      size += chunk.length;
      if (size > MAX_BODY_BYTES) {
        reject(new HttpError(413, "request too large"));
        req.destroy();
      } else chunks.push(chunk);
    });
    req.on("end", () => {
      try {
        resolve(JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}"));
      } catch {
        reject(new HttpError(400, "body must be JSON"));
      }
    });
    req.on("error", reject);
  });
}

const server = http.createServer(async (req, res) => {
  const origin = req.headers.origin;
  const headers = { "Content-Type": "application/json", "Cache-Control": "no-store" };
  if (origin && originAllowed(origin)) {
    Object.assign(headers, { "Access-Control-Allow-Origin": origin, "Access-Control-Allow-Headers": "Content-Type", Vary: "Origin" });
  }
  if (req.method === "OPTIONS") {
    res.writeHead(204, { ...headers, "Access-Control-Allow-Methods": "GET, POST" });
    return res.end();
  }
  const ip = String(req.headers["x-forwarded-for"] || req.socket.remoteAddress).split(",")[0].trim();
  const url = new URL(req.url, "http://localhost");
  try {
    let out;
    if (req.method === "POST" && url.pathname === "/launch/prepare") out = await prepare(await readBody(req), ip);
    else if (req.method === "POST" && url.pathname === "/launch/confirm") out = await confirm(await readBody(req));
    else if (req.method === "POST" && url.pathname === "/fees/collect") out = await collectFees(await readBody(req), ip);
    else if (req.method === "GET" && url.pathname.startsWith("/token/")) out = await token(decodeURIComponent(url.pathname.slice(7)));
    else if (req.method === "GET" && url.pathname === "/tokens") out = tokens(url.searchParams);
    else if (req.method === "GET" && url.pathname === "/stats") out = stats();
    else throw new HttpError(404, "not found");
    res.writeHead(200, headers);
    res.end(JSON.stringify(out));
  } catch (error) {
    const status = error instanceof HttpError ? error.status : 500;
    if (status === 500) console.error(error);
    res.writeHead(status, headers);
    res.end(JSON.stringify({ error: status === 500 ? "internal error" : error.message }));
  }
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`QUBIT launchpad API on 127.0.0.1:${PORT} (${SERVE_DRY ? "DRY-RUN pool, test only" : "IBM pool"})`);
});
setInterval(() => expireReservations().catch((error) => console.error("expiry sweep:", error.message)), 60_000);
