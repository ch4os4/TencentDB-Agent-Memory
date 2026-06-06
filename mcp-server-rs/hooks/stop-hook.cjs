#!/usr/bin/env node
/**
 * TencentDB Memory auto-save Stop Hook
 *
 * Runs every time Claude Code stops responding.
 * Every SAVE_INTERVAL human messages, returns "block" to force Claude
 * to call tdai_capture and save session memories before stopping.
 *
 * State tracked in: ~/.tencentdb-memory/hook_state/<session_id>.json
 */

const fs = require("fs");
const path = require("path");

const SAVE_INTERVAL = 15; // block every N human messages
const STATE_DIR = path.join(
  process.env.HOME || process.env.USERPROFILE || ".",
  ".tencentdb-memory",
  "hook_state"
);

function ensureDir(dir) {
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }
}

function readState(sessionId) {
  const file = path.join(STATE_DIR, `${sessionId}.json`);
  try {
    return JSON.parse(fs.readFileSync(file, "utf-8"));
  } catch {
    return { messageCount: 0, lastSaveAt: 0 };
  }
}

function writeState(sessionId, state) {
  ensureDir(STATE_DIR);
  const file = path.join(STATE_DIR, `${sessionId}.json`);
  fs.writeFileSync(file, JSON.stringify(state));
}

function sanitizeId(id) {
  return (id || "unknown").replace(/[^a-zA-Z0-9_-]/g, "");
}

function countHumanMessages(transcriptPath) {
  if (!transcriptPath) return 0;
  try {
    const content = fs.readFileSync(transcriptPath, "utf-8");
    let count = 0;
    for (const line of content.split("\n")) {
      if (!line.trim()) continue;
      try {
        const entry = JSON.parse(line);
        const msg = entry.message || {};
        if (msg.role === "user") {
          const text =
            typeof msg.content === "string"
              ? msg.content
              : Array.isArray(msg.content)
                ? msg.content.map((b) => b.text || "").join(" ")
                : "";
          if (!text.includes("<command-message>")) {
            count++;
          }
        }
      } catch {}
    }
    return count;
  } catch {
    return 0;
  }
}

function main() {
  let input = "";
  process.stdin.setEncoding("utf-8");
  process.stdin.on("data", (chunk) => (input += chunk));
  process.stdin.on("end", () => {
    try {
      const data = JSON.parse(input || "{}");
      const sessionId = sanitizeId(data.session_id);
      const stopHookActive = data.stop_hook_active || false;
      const transcriptPath = data.transcript_path || "";

      // If another stop hook is already blocking, don't pile on
      if (stopHookActive) {
        process.stdout.write("{}");
        return;
      }

      const msgCount = countHumanMessages(transcriptPath);
      const state = readState(sessionId);

      const messagesSinceLastSave = msgCount - state.lastSaveAt;

      if (messagesSinceLastSave >= SAVE_INTERVAL) {
        // Time to save
        writeState(sessionId, {
          messageCount: msgCount,
          lastSaveAt: msgCount,
        });
        process.stdout.write(
          JSON.stringify({
            decision: "block",
            reason:
              "TencentDB Memory auto-save checkpoint. " +
              "Call tdai_capture with a concise summary of this session's key topics, " +
              "decisions, and code changes as user_content and assistant_content. " +
              "Use session_key matching the current project name.",
          })
        );
      } else {
        // Not yet, update count and let Claude stop
        writeState(sessionId, {
          messageCount: msgCount,
          lastSaveAt: state.lastSaveAt,
        });
        process.stdout.write("{}");
      }
    } catch {
      process.stdout.write("{}");
    }
  });
}

main();
