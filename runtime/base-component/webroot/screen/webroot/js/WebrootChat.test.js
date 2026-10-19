/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// Tests of the chat store (WebrootChat.js) that need no browser: the socket messages on the state (presence, typing, read
// markers, unread and mention counts, the room list), a pushed event merged into a timeline (messages, edits, reactions,
// redactions, pins, threads), the user's own dot and do not disturb, the clear-at choices, the batched presence requests,
// the start of the store, and the account menu's components (they compile, and WebrootVue.qvt.js registers them). The
// account menu and an avatar's dot in the page are the Chromium smoke of Gate 3 (crates/moqui-host/tests/gate3_browser.rs).
// Run: node --test runtime/base-component/webroot/screen/webroot/js/WebrootChat.test.js
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const webroot = path.join(__dirname, "..");
globalThis.moqui = require("./MoquiLib.js");
globalThis.moqui.l10n = require("./MoquiL10n.js");
require("./WebrootChat.js");
vm.runInThisContext(fs.readFileSync(path.join(webroot, "libs/vue/compiler-dom.cjs.js"), "utf8"));
require("./WebrootVue.qvt.js");
const chat = moqui.chat;
const compiler = globalThis.VueCompilerDOM;

const freshState = () => ({ me: null, presence: {}, unread: {}, typing: {}, reads: {}, openRooms: {}, statusDialog: false });
const push = (topic, message) => ({ topic, message });
const event = (fields) => Object.assign({ chatRoomId: "R1", type: "message", senderUserId: "bob", sentDate: 1000, mentions: [], content: {} }, fields);

test("presence and status from the userStatus topic, the user's own with the status chosen", () => {
    const state = freshState();
    assert.deepEqual(chat.reduce(state, push("userStatus", { userId: "bob", presence: "away", statusText: "Lunch", emoji: "🍜" }), "ann", 5),
        { kind: "presence", userId: "bob", presence: "away" });
    assert.equal(state.presence.bob.presence, "away");
    assert.equal(state.presence.bob.statusText, "Lunch");
    chat.reduce(state, push("userStatus", JSON.stringify({ userId: "ann", presence: "offline", self: true, status: "invisible" })), "ann", 6);
    assert.equal(state.me.status, "invisible");
    assert.equal(state.presence.ann.presence, "offline");
    assert.equal(chat.reduce(state, push("userStatus", { presence: "online" }), "ann", 7), null, "no user, nothing");
    assert.equal(chat.reduce(state, { topic: "userStatus" }, "ann", 7), null);
    assert.equal(chat.reduce(state, null, "ann", 7), null);
});

test("typing comes and goes, a message ends it; read markers are kept by room and user", () => {
    const state = freshState();
    chat.reduce(state, push("chatRoom:R1", { kind: "typing", chatRoomId: "R1", userId: "bob", typing: true }), "ann", 1000);
    chat.reduce(state, push("chatRoom:R1", { kind: "typing", chatRoomId: "R1", userId: "cat", typing: true }), "ann", 1000);
    assert.deepEqual(chat.typingUsers(state, "R1", 2000), ["bob", "cat"]);
    assert.deepEqual(chat.typingUsers(state, "R1", 8000), [], "a typing signal lasts a few seconds");
    chat.reduce(state, push("chatRoom:R1", { kind: "typing", chatRoomId: "R1", userId: "cat", typing: false }), "ann", 1500);
    assert.deepEqual(chat.typingUsers(state, "R1", 2000), ["bob"]);
    chat.reduce(state, push("chatRoom:R1", { kind: "event", chatRoomId: "R1", eventSeq: 4, event: event({ eventSeq: 4 }) }), "ann", 1600);
    assert.deepEqual(chat.typingUsers(state, "R1", 2000), []);
    const read = chat.reduce(state, push("chatRoom:R1", { kind: "read", chatRoomId: "R1", userId: "bob", lastReadEventSeq: 4 }), "ann", 1700);
    assert.deepEqual(read, { kind: "read", chatRoomId: "R1", userId: "bob", lastReadEventSeq: 4 });
    assert.deepEqual(state.reads.R1, { bob: 4 });
});

test("unread and mention counts: others' messages in rooms not shown, cleared by a read marker of this user", () => {
    const state = freshState();
    const send = (room, fields) => chat.reduce(state, push("chatRoom:" + room, { kind: "event", chatRoomId: room, event: event(Object.assign({ chatRoomId: room }, fields)) }), "ann", 1);
    send("R1", { eventSeq: 2 });
    send("R1", { eventSeq: 3, mentions: ["ann"] });
    send("R1", { eventSeq: 4, senderUserId: "ann" });
    send("R1", { eventSeq: 5, type: "reaction" });
    send("R2", { eventSeq: 7 });
    assert.deepEqual(chat.unreadOf(state, "R1"), { unread: 2, mentions: 1 });
    assert.equal(chat.unreadTotal(state), 3);
    state.openRooms.R2 = 1;
    send("R2", { eventSeq: 8 });
    assert.deepEqual(chat.unreadOf(state, "R2"), { unread: 1, mentions: 0 }, "a room shown now does not count");
    assert.deepEqual(chat.reduce(state, push("chatUser:ann", { kind: "read", chatRoomId: "R1", lastReadEventSeq: 5 }), "ann", 2),
        { kind: "read", chatRoomId: "R1", userId: "ann", all: false, lastReadEventSeq: 5 });
    assert.deepEqual(chat.unreadOf(state, "R1"), { unread: 0, mentions: 0 });
    chat.reduce(state, push("chatUser:ann", { kind: "read", all: true }), "ann", 3);
    assert.equal(chat.unreadTotal(state), 0);
    send("R3", { eventSeq: 2 });
    assert.deepEqual(chat.reduce(state, push("chatUser:ann", { kind: "room", action: "left", chatRoomId: "R3" }), "ann", 4),
        { kind: "room", chatRoomId: "R3", action: "left" });
    assert.equal(state.unread.R3, undefined);
    assert.equal(chat.reduce(state, push("chatUser:ann", { kind: "other" }), "ann", 5), null);
});

test("a pushed event merged into a timeline: messages in order, edits, reactions, redactions, pins and threads", () => {
    const list = [event({ eventSeq: 2, bodyText: "one", reactions: [], threadReplyCount: 0 }), event({ eventSeq: 5, bodyText: "three" })];
    chat.applyEvent(list, event({ eventSeq: 3, bodyText: "two" }), "ann");
    chat.applyEvent(list, event({ eventSeq: 3, bodyText: "two again" }), "ann");
    assert.deepEqual(list.map((item) => item.bodyText), ["one", "two", "three"], "in order, once");
    assert.equal(list[1].redacted, false);
    chat.applyEvent(list, event({ eventSeq: 6, type: "edit", editsEventSeq: 3, body: "<p>two, edited</p>", bodyText: "two, edited", sentDate: 2000 }), "ann");
    assert.equal(list[1].body, "<p>two, edited</p>");
    assert.equal(list[1].edited, true);
    assert.equal(list.length, 3, "an edit is not a row");
    chat.applyEvent(list, event({ eventSeq: 7, type: "reaction", relatesToEventSeq: 2, senderUserId: "ann", content: { key: "👍" } }), "ann");
    chat.applyEvent(list, event({ eventSeq: 8, type: "reaction", relatesToEventSeq: 2, senderUserId: "bob", content: { key: "👍" } }), "ann");
    chat.applyEvent(list, event({ eventSeq: 9, type: "reaction", relatesToEventSeq: 2, senderUserId: "bob", content: { key: "🎉" } }), "ann");
    assert.deepEqual(list[0].reactions, [{ key: "👍", count: 2, userIds: ["ann", "bob"], mine: true }, { key: "🎉", count: 1, userIds: ["bob"], mine: false }]);
    chat.applyEvent(list, event({ eventSeq: 10, type: "redaction", relatesToEventSeq: 7, senderUserId: "ann", content: { target: "reaction", messageSeq: 2, key: "👍" } }), "ann");
    chat.applyEvent(list, event({ eventSeq: 11, type: "redaction", relatesToEventSeq: 9, senderUserId: "bob", content: { target: "reaction", messageSeq: 2, key: "🎉" } }), "ann");
    assert.deepEqual(list[0].reactions, [{ key: "👍", count: 1, userIds: ["bob"], mine: false }]);
    // a thread reply counts on its root in the main timeline, and is a row of that thread
    chat.applyEvent(list, event({ eventSeq: 12, threadRootEventSeq: 2, bodyText: "reply", sentDate: 3000 }), "ann");
    assert.equal(list.length, 3);
    assert.equal(list[0].threadReplyCount, 1);
    assert.equal(list[0].threadLastDate, 3000);
    const thread = [event({ eventSeq: 2, bodyText: "one" })];
    chat.applyEvent(thread, event({ eventSeq: 12, threadRootEventSeq: 2, bodyText: "reply" }), "ann", 2);
    chat.applyEvent(thread, event({ eventSeq: 13, bodyText: "main timeline" }), "ann", 2);
    assert.deepEqual(thread.map((item) => item.bodyText), ["one", "reply"]);
    // a pin marks its message and is a row too; a redaction removes the content
    chat.applyEvent(list, event({ eventSeq: 14, type: "pin", relatesToEventSeq: 5, senderUserId: "ann", content: { pinned: true } }), "ann");
    assert.equal(list[2].pinned, true);
    assert.equal(list[3].type, "pin");
    chat.applyEvent(list, event({ eventSeq: 15, type: "redaction", relatesToEventSeq: 5, content: { target: "message" } }), "ann");
    assert.equal(list[2].redacted, true);
    assert.equal(list[2].body, null);
    assert.equal(list[2].pinned, false);
    assert.equal(list.length, 4, "a redaction is not a row");
    assert.equal(chat.applyEvent(null, event({ eventSeq: 1 })), null);
});

test("the user's own dot and do not disturb: no toasts, the notifications still come", () => {
    const saved = { state: chat.state, vm: chat.vm, me: chat.me };
    chat.state = freshState();
    let display = true;
    chat.vm = { notificationClient: { disableDisplay: () => { display = false; }, enableDisplay: () => { display = true; } } };
    chat.me = "ann";
    assert.equal(chat.myPresence(), "");
    chat.setMe({ status: "online", presence: "online", autoAwayMinutes: 15 });
    assert.equal(chat.myPresence(), "online");
    assert.equal(chat.autoAwayMinutes, 15);
    assert.equal(chat.presenceOf("ann"), "online");
    chat.setMe({ status: "dnd", presence: "dnd" });
    assert.equal(chat.myPresence(), "dnd");
    assert.equal(chat.quiet(), true);
    assert.equal(display, false, "toasts are off");
    chat.receive(push("userStatus", { userId: "ann", presence: "online", self: true, status: "online" }));
    assert.equal(chat.quiet(), false);
    assert.equal(display, true);
    chat.setMe({ status: "invisible", presence: "offline" });
    assert.equal(chat.myPresence(), "offline", "invisible is a grey dot");
    chat.setMe({ status: "online", presence: "away" });
    assert.equal(chat.myPresence(), "away", "idle: away");
    chat.state.presence.bob = { presence: "dnd" };
    assert.equal(chat.presenceOf("bob"), "dnd");
    assert.equal(chat.presenceOf("cat"), "", "not known yet");
    assert.equal(chat.presenceOf(""), "");
    Object.assign(chat, saved);
});

test("the clear-at choices and the idle time", () => {
    const now = new Date(2026, 9, 21, 10, 30, 0).getTime(); // a Wednesday
    assert.equal(chat.clearAt(0, now), null);
    assert.equal(chat.clearAt(30, now), now + 30 * 60000);
    assert.equal(chat.clearAt("240", now), now + 240 * 60000);
    const today = new Date(chat.clearAt("today", now));
    assert.deepEqual([today.getDate(), today.getHours(), today.getMinutes()], [21, 23, 59]);
    const week = new Date(chat.clearAt("week", now));
    assert.deepEqual([week.getDay(), week.getDate(), week.getHours()], [0, 25, 23]);
    assert.deepEqual(chat.clearOptions().map((option) => option.value), [0, 30, 60, 240, "today", "week"]);
    assert.equal(chat.clearOptions({ today: "Hoy" })[4].label, "Hoy");
    assert.equal(chat.idleSeconds(1000, 62500), 61);
    assert.equal(chat.idleSeconds(5000, 1000), 0);
    assert.equal(chat.backFromIdle(600, 10), true);
    assert.equal(chat.backFromIdle(599, 10), false);
    assert.equal(chat.backFromIdle(700, 30), false);
    assert.equal(chat.dot("invisible"), "offline");
    assert.equal(chat.dot("nope"), "");
});

test("the tray's Chat tab reloads for an event no screen listens to, never for typing or a read marker", () => {
    assert.equal(chat.trayReloads(push("chatRoom:R1", { kind: "typing" })), false);
    assert.equal(chat.trayReloads(push("chatRoom:R1", { kind: "read" })), false);
    assert.equal(chat.trayReloads(push("chatRoom:R1", { kind: "event" })), true);
    assert.equal(chat.trayReloads({ topic: "chatRoom:R1" }), true);
    const off = chat.on("event", () => {});
    assert.equal(chat.trayReloads(push("chatRoom:R1", { kind: "event" })), false, "a live room list or room");
    off();
    assert.equal(chat.trayReloads(push("chatRoom:R1", { kind: "event" })), true);
});

test("listeners hear the reduced messages; one that throws does not stop the others", () => {
    const saved = { state: chat.state, me: chat.me };
    chat.state = freshState();
    chat.me = "ann";
    const heard = [];
    const offs = [chat.on("event", () => { throw new Error("broken listener"); }), chat.on("event", (payload) => heard.push(payload.event.eventSeq)),
        chat.on("typing", (payload) => heard.push("typing " + payload.userId))];
    const errors = console.error;
    console.error = () => {};
    chat.receive(push("chatRoom:R1", { kind: "event", chatRoomId: "R1", event: event({ eventSeq: 9 }) }));
    chat.receive(push("chatRoom:R1", { kind: "typing", chatRoomId: "R1", userId: "bob", typing: true }));
    chat.receive(push("shellCounts", { chat: 1 }));
    console.error = errors;
    assert.deepEqual(heard, [9, "typing bob"]);
    offs.forEach((off) => off());
    Object.assign(chat, saved);
});

test("the store starts once: its topics, the user's status, the counts and a heartbeat; presence asks go as one request", async () => {
    const requests = [];
    const listeners = [];
    const original = moqui.request;
    moqui.request = (url, options) => {
        requests.push({ url, method: (options && options.method) || "GET", params: options && options.params });
        let json = {};
        if (url.endsWith("/userStatus") || url.endsWith("/userStatusHeartbeat")) json = { myStatus: { status: "online", presence: "online", autoAwayMinutes: 10 } };
        if (url.endsWith("/chatUnread")) json = { roomCounts: { R1: { unread: 2, mentions: 1 } } };
        if (url.endsWith("/userStatuses")) json = { statusMap: { bob: { presence: "away" }, cat: { presence: "online" } } };
        return Promise.resolve({ json });
    };
    const client = { registerListener: (topic, fn) => listeners.push(topic), disableDisplay() {}, enableDisplay() {} };
    chat.state = freshState();
    chat.start({ userId: "ann", appRootPath: "", notificationClient: client });
    chat.start({ userId: "ann", appRootPath: "", notificationClient: client });
    clearInterval(chat.beatTimer);
    assert.deepEqual(listeners, ["ALL", "chatUser:ann", "userStatus"]);
    await new Promise((resolve) => setTimeout(resolve, 10));
    assert.deepEqual(requests.map((request) => request.method + " " + request.url), ["GET /userStatus", "GET /chatUnread", "POST /userStatusHeartbeat"]);
    assert.equal(typeof requests[2].params.idleSeconds, "number");
    assert.deepEqual(chat.unreadOf(chat.state, "R1"), { unread: 2, mentions: 1 });
    assert.equal(chat.myPresence(), "online");
    // the avatars of a page ask at once: one request, and not again soon
    chat.watchUsers(["bob", "ann"]);
    chat.watchUsers(["cat", "bob"]);
    await new Promise((resolve) => setTimeout(resolve, 80));
    const asked = requests.filter((request) => request.url.endsWith("/userStatuses"));
    assert.equal(asked.length, 1);
    assert.equal(asked[0].params.userIds, "bob,cat");
    assert.equal(chat.presenceOf("bob"), "away");
    chat.watchUsers(["bob"]);
    await new Promise((resolve) => setTimeout(resolve, 80));
    assert.equal(requests.filter((request) => request.url.endsWith("/userStatuses")).length, 1);
    // a room shown: its topic, and its count read
    chat.openRoom("R1");
    assert.deepEqual(chat.unreadOf(chat.state, "R1"), { unread: 0, mentions: 0 });
    assert.equal(listeners[listeners.length - 1], "chatRoom:R1");
    chat.closeRoom("R1");
    assert.equal(chat.state.openRooms.R1, undefined);
    // typing goes at most every 3 seconds, and once when it stops
    chat.typing("R1");
    chat.typing("R1");
    chat.typing("R1", false);
    chat.typing("R1", false);
    const typed = requests.filter((request) => request.url.endsWith("/chatTyping"));
    assert.deepEqual(typed.map((request) => request.params.typing), [true, false]);
    // the status of the account menu, with a clear-at time
    await chat.setStatus({ status: "dnd", statusText: "Focus", clearAt: 30 });
    const set = requests.filter((request) => request.url.endsWith("/setUserStatus"))[0];
    assert.equal(set.method, "POST");
    assert.equal(set.params.status, "dnd");
    assert.ok(set.params.clearAtDate > Date.now(), "clear-at as epoch millis");
    assert.equal(set.params.clearAt, undefined);
    moqui.request = original;
});

test("the account menu's components compile, and WebrootVue.qvt.js registers them before the shell", () => {
    ["m-user-status-menu", "m-user-status-dialog"].forEach((name) => {
        const comp = moqui.qvt.components[name];
        assert.ok(comp && comp.template, name);
        const errors = [];
        compiler.compile(comp.template, { mode: "function", onError: (err) => errors.push(err.message) });
        assert.deepEqual(errors, [], name);
    });
    const shell = moqui.qvt.components["m-qapps-root"].template;
    assert.match(shell, /<m-user-status-menu v-if="chatStatus"><\/m-user-status-menu>/);
    assert.match(shell, /<m-user-status-dialog v-if="chatStatus"><\/m-user-status-dialog>/);
    assert.match(shell, /<m-avatar :name="userFullName \|\| username" :user-id="userId"/);
    const avatar = moqui.qvt.components["m-avatar"];
    assert.ok(avatar.props.userId);
    // an avatar's dot: its presence, else the store's for its user
    const saved = { state: chat.state, me: chat.me };
    chat.state = freshState();
    chat.state.presence.bob = { presence: "away" };
    assert.equal(avatar.computed.shownPresence.call({ presence: "", userId: "bob" }), "away");
    assert.equal(avatar.computed.shownPresence.call({ presence: "dnd", userId: "bob" }), "dnd");
    assert.equal(avatar.computed.shownPresence.call({ presence: "" }), "");
    Object.assign(chat, saved);
    const menu = moqui.qvt.components["m-user-status-menu"];
    assert.deepEqual(menu.computed.options.call({ labels: chat.labels }).map((option) => option.value + ":" + option.dot),
        ["online:online", "away:away", "dnd:dnd", "invisible:offline"]);
});
