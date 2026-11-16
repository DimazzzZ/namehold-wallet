#!/usr/bin/env node
// nostr-req.mjs — read events from a Nostr relay, for QA (read-only: it sends
// only REQ and CLOSE, never an event). No dependencies: Node 22+'s WebSocket.
//
// Usage:
//   node scripts/nostr-req.mjs <relay-url> '<filter JSON>' [--full]
//
// Examples:
//   node scripts/nostr-req.mjs ws://127.0.0.1:7777 '{"kinds":[37491]}'
//   node scripts/nostr-req.mjs ws://127.0.0.1:7777 '{"kinds":[7491],"#p":["<x-only key>"]}' --full
//
// Prints one line per event, oldest first: kind, created_at, the author
// (64 hex), the tags, and the content (its first 80 characters, or all
// of it with --full); then a line "N events". Exits 1 when the relay does
// not answer within 15 s or closes the read.

const [url, filterText, flag] = process.argv.slice(2);
if (!url || !filterText) {
  console.error("usage: node scripts/nostr-req.mjs <relay-url> '<filter JSON>' [--full]");
  process.exit(2);
}
const filter = JSON.parse(filterText);
const full = flag === "--full";
const sub = "qa" + Math.random().toString(16).slice(2, 10);
const events = [];
const ws = new WebSocket(url);
const timer = setTimeout(() => {
  console.error(`no answer from ${url} within 15 s`);
  process.exit(1);
}, 15000);

ws.onerror = (e) => {
  console.error(`cannot read ${url}: ${e.message ?? e.type}`);
  process.exit(1);
};
ws.onopen = () => ws.send(JSON.stringify(["REQ", sub, filter]));
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg[1] !== sub) return;
  if (msg[0] === "EVENT") events.push(msg[2]);
  if (msg[0] === "CLOSED") {
    console.error(`the relay closed the read: ${msg[2]}`);
    process.exit(1);
  }
  if (msg[0] === "EOSE") {
    clearTimeout(timer);
    ws.send(JSON.stringify(["CLOSE", sub]));
    events.sort((a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id));
    for (const e of events) {
      const content = full ? e.content : e.content.slice(0, 80);
      console.log(
        `${e.kind} ${e.created_at} ${e.pubkey} ${JSON.stringify(e.tags)} ${JSON.stringify(content)}`,
      );
    }
    console.log(`${events.length} events`);
    ws.close();
    process.exit(0);
  }
};
