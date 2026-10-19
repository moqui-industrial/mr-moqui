/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

/*
 * Chat and presence in the /qapps shell (IronStone design-drawers-commons §1.8, §13; impl-drawers-commons M10): moqui.chat, a
 * small store over the notification socket, and the account menu's status. It keeps
 *   - presence: the presence and status of the users the page shows (m-avatar with a user-id draws the dot from it),
 *   - me: the current user's own status, set from the account menu, with the heartbeat that keeps it (auto-away from idle),
 *   - unread: each room's unread and mention counts, typing: who is typing in each room, reads: the members' read markers,
 * and hands the chat pushes to the screens that listen (moqui.chat.on). The server side is org.moqui.impl.ChatServices; the
 * topics and their messages are described there and in js/README.md, "Chat store and presence".
 *
 * Loaded after MoquiLib.js and before WebrootVue.qvt.js (qapps.xml): its components are in moqui.qvtComponents, which
 * WebrootVue.qvt.js registers, and the shell root calls moqui.chat.start(root) once mounted. Runs under Node for its tests
 * (WebrootChat.test.js): nothing here touches the page until start.
 */
(function (root) {
    "use strict";

    const moqui = root.moqui;
    const reactive = (obj) => (root.Vue && root.Vue.reactive ? root.Vue.reactive(obj) : obj);

    // A presence as the dot draws it: online, away, dnd or offline; invisible is offline (to others and on one's own dot).
    const STATUSES = ["online", "away", "dnd", "invisible"];
    const DOT = { online: "online", away: "away", dnd: "dnd", invisible: "offline", offline: "offline" };
    const TYPING_FOR = 6000;
    const BEAT_EVERY = 60 * 1000;
    const STATUS_TTL = 5 * 60 * 1000;

    const chat = {
        labels: { statusOnline: "Online", statusAway: "Away", statusDnd: "Do not disturb", statusInvisible: "Invisible", statusOffline: "Offline",
            setStatus: "Set a status message", clearStatus: "Clear status message", statusText: "What's your status?", emoji: "Emoji",
            clearAfter: "Clear after", dontClear: "Don't clear", minutes30: "30 minutes", hour1: "1 hour", hours4: "4 hours", today: "Today",
            thisWeek: "This week", save: "Save", cancel: "Cancel", status: "Status" },

        // ---- the pure parts (WebrootChat.test.js) ----

        statuses: STATUSES,
        // the dot of a presence or status, "" for none
        dot(presence) { return DOT[presence] || ""; },
        // the clear-at choices of the status dialog: minutes from now, or the end of today or of this week
        clearOptions(labels) {
            const text = Object.assign({}, chat.labels, labels);
            return [{ value: 0, label: text.dontClear }, { value: 30, label: text.minutes30 }, { value: 60, label: text.hour1 },
                { value: 240, label: text.hours4 }, { value: "today", label: text.today }, { value: "week", label: text.thisWeek }];
        },
        // the time a clear-at choice comes to (epoch millis, in the browser's zone for today and the week), null for none
        clearAt(option, now) {
            const at = new Date(now);
            if (option === "today") { at.setHours(23, 59, 59, 999); return at.getTime(); }
            if (option === "week") {
                // the end of Sunday, the week of ISO 8601
                const toSunday = (7 - at.getDay()) % 7;
                at.setDate(at.getDate() + toSunday);
                at.setHours(23, 59, 59, 999);
                return at.getTime();
            }
            const minutes = Number(option);
            return minutes > 0 ? now + minutes * 60000 : null;
        },
        // whole seconds since the last input
        idleSeconds(lastInput, now) { return Math.max(0, Math.floor((now - lastInput) / 1000)); },
        // whether input now, after the last beat reported this idle, should beat at once (the user is back from away)
        backFromIdle(lastBeatIdle, autoAwayMinutes) { return lastBeatIdle >= (Number(autoAwayMinutes) || 10) * 60; },

        // A pushed event merged into a timeline (a list of events as get#Timeline answers them, oldest first): a message of
        // the main timeline is added once (a thread reply only counts on its root, unless the list is that thread), an
        // edit replaces its message's body, a reaction is added to its message, a redaction removes a message's content or
        // takes a reaction back, a pin marks its message. Returns the list (the same array, changed in place).
        applyEvent(list, event, me, threadRoot) {
            if (!Array.isArray(list) || !event) return list;
            const seq = Number(event.eventSeq);
            const find = (target) => list.find((item) => Number(item.eventSeq) === Number(target));
            const type = event.type || "";
            if (type === "edit") {
                const target = find(event.editsEventSeq);
                if (target && !target.redacted) {
                    target.body = event.body; target.bodyText = event.bodyText; target.mentions = event.mentions || [];
                    target.edited = true; target.editedDate = event.sentDate;
                }
                return list;
            }
            if (type === "reaction") {
                const target = find(event.relatesToEventSeq);
                const key = event.content && event.content.key;
                if (!target || !key) return list;
                target.reactions = target.reactions || [];
                let entry = target.reactions.find((reaction) => reaction.key === key);
                if (!entry) { entry = { key, count: 0, userIds: [], mine: false }; target.reactions.push(entry); }
                if (entry.userIds.indexOf(event.senderUserId) < 0) {
                    entry.userIds.push(event.senderUserId);
                    entry.count++;
                    if (event.senderUserId === me) entry.mine = true;
                }
                return list;
            }
            if (type === "redaction") {
                const content = event.content || {};
                if (content.target === "reaction") {
                    const target = find(content.messageSeq);
                    const entry = target && (target.reactions || []).find((reaction) => reaction.key === content.key);
                    if (entry) {
                        const index = entry.userIds.indexOf(event.senderUserId);
                        if (index >= 0) { entry.userIds.splice(index, 1); entry.count--; }
                        if (event.senderUserId === me) entry.mine = false;
                        if (entry.count <= 0) target.reactions.splice(target.reactions.indexOf(entry), 1);
                    }
                } else {
                    const target = find(event.relatesToEventSeq);
                    if (target) Object.assign(target, { redacted: true, body: null, bodyText: null, attachments: [], records: [], reactions: [],
                        mentions: [], pinned: false, edited: false, redactedDate: event.sentDate });
                    if (target && target.threadRootEventSeq != null && threadRoot == null) {
                        const rootItem = find(target.threadRootEventSeq);
                        if (rootItem) rootItem.threadReplyCount = Math.max(0, (rootItem.threadReplyCount || 0) - 1);
                    }
                }
                return list;
            }
            if (type === "pin") {
                const target = find(event.relatesToEventSeq);
                if (target) { target.pinned = !!(event.content && event.content.pinned); target.pinnedByUserId = target.pinned ? event.senderUserId : null; }
                // the pin is also a row of the timeline
            }
            if (find(seq)) return list;
            const inThread = event.threadRootEventSeq != null;
            if (inThread && Number(threadRoot) !== Number(event.threadRootEventSeq)) {
                const rootItem = find(event.threadRootEventSeq);
                if (rootItem) { rootItem.threadReplyCount = (rootItem.threadReplyCount || 0) + 1; rootItem.threadLastDate = event.sentDate; }
                return list;
            }
            if (threadRoot != null && !inThread && seq !== Number(threadRoot)) return list;
            const item = Object.assign({ edited: false, redacted: false, pinned: false, reactions: [], threadReplyCount: 0, replyTo: null }, event);
            let at = list.length;
            while (at > 0 && Number(list[at - 1].eventSeq) > seq) at--;
            list.splice(at, 0, item);
            return list;
        },

        // A message of the socket on the state: presence (userStatus), the room events and signals (chatRoom:<id>) and the
        // user's room list (chatUser:<me>). Returns what it was ({kind, chatRoomId, ...}) for the listeners, or null.
        reduce(state, jsonObj, me, now) {
            const topic = jsonObj && jsonObj.topic ? String(jsonObj.topic) : "";
            let message = jsonObj ? jsonObj.message : null;
            if (moqui.isString(message)) { try { message = JSON.parse(message); } catch (err) { message = null; } }
            if (!topic || !moqui.isPlainObject(message)) return null;
            if (topic === "userStatus") {
                if (!message.userId) return null;
                const entry = { presence: message.presence || "offline", statusText: message.statusText || null, emoji: message.emoji || null,
                    clearAtDate: message.clearAtDate || null, inCall: !!message.inCall, at: now };
                if (message.userId === me) {
                    state.me = Object.assign({}, state.me || {}, entry, message.self && message.status ? { status: message.status } : {});
                    if (message.self && message.status === "invisible") entry.presence = "offline";
                }
                state.presence[message.userId] = entry;
                return { kind: "presence", userId: message.userId, presence: entry.presence };
            }
            if (topic.indexOf("chatRoom:") === 0) {
                const roomId = message.chatRoomId || topic.slice("chatRoom:".length);
                if (message.kind === "typing") {
                    const typing = state.typing[roomId] = Object.assign({}, state.typing[roomId]);
                    if (message.typing) typing[message.userId] = now + TYPING_FOR;
                    else delete typing[message.userId];
                    return { kind: "typing", chatRoomId: roomId, userId: message.userId, typing: !!message.typing };
                }
                if (message.kind === "read") {
                    state.reads[roomId] = Object.assign({}, state.reads[roomId], { [message.userId]: message.lastReadEventSeq });
                    return { kind: "read", chatRoomId: roomId, userId: message.userId, lastReadEventSeq: message.lastReadEventSeq };
                }
                if (message.kind === "event" && message.event) {
                    const event = message.event;
                    // who sent a message has stopped typing
                    if (event.senderUserId && state.typing[roomId] && state.typing[roomId][event.senderUserId]) {
                        const typing = state.typing[roomId] = Object.assign({}, state.typing[roomId]);
                        delete typing[event.senderUserId];
                    }
                    if (event.type === "message" && event.senderUserId !== me && !state.openRooms[roomId]) {
                        const counts = state.unread[roomId] = Object.assign({ unread: 0, mentions: 0 }, state.unread[roomId]);
                        counts.unread++;
                        if ((event.mentions || []).indexOf(me) >= 0) counts.mentions++;
                    }
                    return { kind: "event", chatRoomId: roomId, event };
                }
                return null;
            }
            if (topic.indexOf("chatUser:") === 0) {
                if (message.kind === "read") {
                    if (message.all) Object.keys(state.unread).forEach((roomId) => { state.unread[roomId] = { unread: 0, mentions: 0 }; });
                    else if (message.chatRoomId) state.unread[message.chatRoomId] = { unread: 0, mentions: 0 };
                    return { kind: "read", chatRoomId: message.chatRoomId || null, userId: me, all: !!message.all, lastReadEventSeq: message.lastReadEventSeq };
                }
                if (message.kind === "room") {
                    if (message.action === "left") delete state.unread[message.chatRoomId];
                    return { kind: "room", chatRoomId: message.chatRoomId, action: message.action };
                }
            }
            return null;
        },
        // the users still typing in a room
        typingUsers(state, roomId, now) {
            const typing = state.typing[roomId] || {};
            return Object.keys(typing).filter((userId) => typing[userId] > now);
        },
        // the room's unread counts and the total of every room
        unreadOf(state, roomId) { return state.unread[roomId] || { unread: 0, mentions: 0 }; },
        unreadTotal(state) { return Object.keys(state.unread).reduce((sum, roomId) => sum + (state.unread[roomId].unread || 0), 0); },

        // ---- the store of the page ----

        state: reactive({ me: null, presence: {}, unread: {}, typing: {}, reads: {}, openRooms: {}, statusDialog: false }),
        listeners: {},
        vm: null,
        started: false,
        watched: {},
        pendingUsers: [],
        lastInput: Date.now(),
        lastBeatIdle: 0,

        // The page's store starts once the shell is mounted: its socket listener, the user's status, the counts, the idle
        // tracking and the heartbeat. Without a signed in user it does nothing.
        start(vm) {
            if (chat.started || !vm || !vm.userId) return;
            chat.started = true;
            chat.vm = vm;
            chat.me = vm.userId;
            const client = vm.notificationClient;
            if (client) {
                client.registerListener("ALL", chat.receive);
                // the topics of the store, named so a page that subscribes topic by topic gets them too
                ["chatUser:" + chat.me, "userStatus"].forEach((topic) => client.registerListener(topic, chat.noop));
            }
            const prefs = (moqui.config && moqui.config.preferences) || {};
            chat.autoAwayMinutes = Number(prefs.CHAT_AUTO_AWAY_MINUTES) || 10;
            chat.request("/userStatus").then((json) => { if (json && json.myStatus) chat.setMe(json.myStatus); });
            chat.request("/chatUnread").then((json) => {
                const counts = json && moqui.isPlainObject(json.roomCounts) ? json.roomCounts : {};
                Object.keys(counts).forEach((roomId) => { chat.state.unread[roomId] = { unread: counts[roomId].unread || 0, mentions: counts[roomId].mentions || 0 }; });
            });
            const doc = root.document;
            if (doc && doc.addEventListener) {
                const input = () => chat.onInput();
                ["keydown", "pointerdown", "wheel", "touchstart", "focusin"].forEach((name) => doc.addEventListener(name, input, { passive: true, capture: true }));
                let moved = 0;
                doc.addEventListener("mousemove", () => { const now = Date.now(); if (now - moved > 5000) { moved = now; chat.onInput(); } }, { passive: true });
                doc.addEventListener("visibilitychange", () => { if (doc.visibilityState === "visible") chat.onInput(); });
            }
            chat.beat();
            chat.beatTimer = setInterval(() => chat.beat(), BEAT_EVERY);
            // the avatars mounted before the store started
            chat.watchUsers([]);
        },
        noop() {},
        // a GET or POST to a webroot transition of the store, its JSON (null when it fails, which is logged)
        request(path, params, method) {
            const base = (chat.vm && chat.vm.appRootPath) || "";
            return moqui.request(base + path, { method: method || "GET", params }).then((result) => result.json || null,
                (err) => { console.info("chat " + path + " failed", err && err.status); return null; });
        },
        receive(jsonObj) {
            const done = chat.reduce(chat.state, jsonObj, chat.me, Date.now());
            if (!done) return;
            if (done.kind === "presence" && done.userId === chat.me) chat.applyQuiet();
            chat.emit(done.kind, done);
        },
        // listeners of the screens: kind is event, typing, read, room or presence; returns the function that removes it
        on(kind, fn) {
            (chat.listeners[kind] = chat.listeners[kind] || []).push(fn);
            return () => { const list = chat.listeners[kind] || []; const index = list.indexOf(fn); if (index >= 0) list.splice(index, 1); };
        },
        emit(kind, payload) {
            (chat.listeners[kind] || []).slice().forEach((fn) => {
                try { fn(payload); } catch (err) { console.error("chat listener failed", err); }
            });
        },
        // Whether a chat push should reload the tray's Chat tab: not for a typing or read signal, and not when a screen
        // listens to the events itself (a live room list or room).
        trayReloads(jsonObj) {
            const message = jsonObj && moqui.isPlainObject(jsonObj.message) ? jsonObj.message : {};
            if (message.kind === "typing" || message.kind === "read") return false;
            return !((chat.listeners.event || []).length || (chat.listeners.room || []).length);
        },
        // a room shown now: its events count as read there; the socket is told of its topic
        openRoom(roomId) {
            if (!roomId) return;
            chat.state.openRooms[roomId] = (chat.state.openRooms[roomId] || 0) + 1;
            chat.state.unread[roomId] = { unread: 0, mentions: 0 };
            if (chat.vm && chat.vm.notificationClient) chat.vm.notificationClient.registerListener("chatRoom:" + roomId, chat.noop);
        },
        closeRoom(roomId) {
            if (!chat.state.openRooms[roomId]) return;
            if (--chat.state.openRooms[roomId] <= 0) delete chat.state.openRooms[roomId];
        },
        // typing in a room's composer: told at most every 3 seconds, and once when it stops
        typing(roomId, typing) {
            const now = Date.now();
            chat.typingSent = chat.typingSent || {};
            const stop = typing === false;
            if (!stop && chat.typingSent[roomId] && now - chat.typingSent[roomId] < 3000) return;
            if (stop && !chat.typingSent[roomId]) return;
            chat.typingSent[roomId] = stop ? 0 : now;
            chat.request("/chatTyping", { chatRoomId: roomId, typing: !stop }, "POST");
        },
        typingIn(roomId) { return chat.typingUsers(chat.state, roomId, Date.now()); },

        // ---- presence ----

        // the dot of a user: the presence the store has, "" while it is not known (asked for by watchUsers)
        presenceOf(userId) {
            if (!userId) return "";
            if (userId === chat.me && chat.state.me) return chat.myPresence();
            const entry = chat.state.presence[userId];
            return entry ? DOT[entry.presence] || "offline" : "";
        },
        statusOf(userId) { return chat.state.presence[userId] || null; },
        // my own dot: what I chose (invisible is a grey dot), else what the server says
        myPresence() {
            const me = chat.state.me;
            if (!me) return "";
            if (me.status === "invisible") return "offline";
            if (me.status === "dnd" || me.status === "away") return me.status;
            return DOT[me.presence] === "offline" ? "online" : DOT[me.presence] || "online";
        },
        // do not disturb: no toasts and no chat sounds; the notifications are still recorded and counted
        quiet() { return !!chat.state.me && chat.state.me.status === "dnd"; },
        applyQuiet() {
            const client = chat.vm && chat.vm.notificationClient;
            if (!client) return;
            if (chat.quiet()) client.disableDisplay(); else client.enableDisplay();
        },
        setMe(myStatus) {
            chat.state.me = Object.assign({}, myStatus);
            if (myStatus.autoAwayMinutes) chat.autoAwayMinutes = myStatus.autoAwayMinutes;
            chat.state.presence[chat.me] = { presence: myStatus.presence, statusText: myStatus.statusText || null, emoji: myStatus.emoji || null,
                clearAtDate: myStatus.clearAtDate || null, inCall: !!myStatus.inCall, at: Date.now() };
            chat.applyQuiet();
        },
        // ask for these users' presence, once each in five minutes; the asks of the same moment go as one request
        watchUsers(userIds) {
            const now = Date.now();
            (userIds || []).forEach((userId) => {
                if (!userId || userId === chat.me) return;
                if (chat.watched[userId] && now - chat.watched[userId] < STATUS_TTL) return;
                chat.watched[userId] = now;
                chat.pendingUsers.push(userId);
            });
            if (!chat.pendingUsers.length || chat.watchTimer || !chat.vm) return;
            chat.watchTimer = setTimeout(() => {
                chat.watchTimer = null;
                const ids = chat.pendingUsers.splice(0, chat.pendingUsers.length).filter((userId) => userId !== chat.me);
                if (!ids.length) return;
                chat.request("/userStatuses", { userIds: ids.join(",") }).then((json) => {
                    const map = json && moqui.isPlainObject(json.statusMap) ? json.statusMap : {};
                    Object.keys(map).forEach((userId) => { chat.state.presence[userId] = Object.assign({ at: Date.now() }, map[userId]); });
                });
            }, 50);
        },
        // the account menu: a status, a status text with an emoji and a clear-at choice, or clearing the text
        setStatus(params) {
            const send = Object.assign({}, params);
            if (send.clearAt !== undefined) {
                const at = chat.clearAt(send.clearAt, Date.now());
                delete send.clearAt;
                if (at) send.clearAtDate = at;
            }
            return chat.request("/setUserStatus", send, "POST").then((json) => {
                if (json && json.myStatus) chat.setMe(json.myStatus);
                return json;
            });
        },
        openStatusDialog() { chat.state.statusDialog = true; },
        onInput() {
            const now = Date.now();
            const wasIdle = chat.idleSeconds(chat.lastInput, now);
            chat.lastInput = now;
            // back from away: beat now so the others see it at once
            if (chat.backFromIdle(Math.max(chat.lastBeatIdle, wasIdle), chat.autoAwayMinutes)) chat.beat();
        },
        beat() {
            if (!chat.vm) return;
            const idle = chat.idleSeconds(chat.lastInput, Date.now());
            chat.lastBeatIdle = idle;
            chat.request("/userStatusHeartbeat", { idleSeconds: idle }, "POST").then((json) => { if (json && json.myStatus) chat.setMe(json.myStatus); });
        }
    };
    moqui.chat = chat;

    // ---- the components the shell writes (registered by WebrootVue.qvt.js) ----

    const labelsOf = (vm) => Object.assign({}, chat.labels, vm.$root && vm.$root.labels);
    const components = moqui.qvtComponents = moqui.qvtComponents || {};

    // The status part of the account menu: the current status with its text, the four statuses, and the status message.
    components["m-user-status-menu"] = {
        name: "mUserStatusMenu",
        computed: {
            labels() { return labelsOf(this); },
            me() { return chat.state.me || {}; },
            current() { return this.me.status || "online"; },
            currentDot() { return chat.myPresence() || "offline"; },
            currentLabel() { return this.labels["status" + this.current.charAt(0).toUpperCase() + this.current.slice(1)] || this.current; },
            options() {
                return STATUSES.map((status) => ({ value: status, dot: DOT[status], label: this.labels["status" + status.charAt(0).toUpperCase() + status.slice(1)] }));
            }
        },
        methods: {
            choose(status) { chat.setStatus({ status }); },
            openDialog() { chat.openStatusDialog(); },
            clearText() { chat.setStatus({ clearStatus: true }); }
        },
        template:
            '<div class="m-user-status">' +
                '<q-item id="m-status-current" dense>' +
                    '<q-item-section avatar class="m-status-avatar"><span class="m-status-dot" :class="\'m-presence-\' + currentDot"></span></q-item-section>' +
                    '<q-item-section><q-item-label>{{currentLabel}}</q-item-label>' +
                        '<q-item-label v-if="me.statusText || me.emoji" caption class="m-status-text">{{me.emoji ? me.emoji + \' \' : \'\'}}{{me.statusText}}</q-item-label></q-item-section>' +
                '</q-item>' +
                '<q-item v-for="option in options" :key="option.value" dense clickable v-close-popup :active="current === option.value" :data-status="option.value" @click="choose(option.value)">' +
                    '<q-item-section avatar class="m-status-avatar"><span class="m-status-dot" :class="\'m-presence-\' + option.dot"></span></q-item-section>' +
                    '<q-item-section>{{option.label}}</q-item-section>' +
                '</q-item>' +
                '<q-item dense clickable v-close-popup data-status="message" @click="openDialog">' +
                    '<q-item-section avatar class="m-status-avatar"><q-icon name="sym_o_edit" size="xs"></q-icon></q-item-section>' +
                    '<q-item-section>{{labels.setStatus}}</q-item-section>' +
                '</q-item>' +
                '<q-item v-if="me.statusText || me.emoji" dense clickable v-close-popup data-status="clear" @click="clearText">' +
                    '<q-item-section avatar class="m-status-avatar"><q-icon name="sym_o_close" size="xs"></q-icon></q-item-section>' +
                    '<q-item-section>{{labels.clearStatus}}</q-item-section>' +
                '</q-item>' +
            '</div>'
    };

    // The status message dialog of the account menu, outside the menu so it outlives it.
    components["m-user-status-dialog"] = {
        name: "mUserStatusDialog",
        data() { return { text: "", emoji: "", clear: 0, saving: false }; },
        computed: {
            labels() { return labelsOf(this); },
            open: { get() { return chat.state.statusDialog; }, set(value) { chat.state.statusDialog = !!value; } },
            clearOptions() { return chat.clearOptions(this.labels); }
        },
        watch: {
            open(value) {
                if (!value) return;
                const me = chat.state.me || {};
                this.text = me.statusText || "";
                this.emoji = me.emoji || "";
                this.clear = 0;
            }
        },
        methods: {
            save() {
                this.saving = true;
                const params = { statusText: this.text, emoji: this.emoji };
                if (this.clear) params.clearAt = this.clear;
                else params.clearAfterMinutes = 0;
                chat.setStatus(params).then(() => { this.saving = false; this.open = false; });
            }
        },
        template:
            '<q-dialog v-model="open">' +
                '<q-card style="min-width:320px;max-width:96vw;">' +
                    '<q-card-section class="text-subtitle1">{{labels.setStatus}}</q-card-section>' +
                    '<q-card-section class="q-pt-none q-gutter-y-sm">' +
                        '<div class="row no-wrap q-gutter-x-sm">' +
                            '<q-input v-model="emoji" dense outlined stack-label :label="labels.emoji" maxlength="16" style="width:84px;" name="emoji"></q-input>' +
                            '<q-input v-model="text" dense outlined stack-label autofocus class="col" :label="labels.statusText" maxlength="100" name="statusText" @keyup.enter="save"></q-input>' +
                        '</div>' +
                        '<q-select v-model="clear" :options="clearOptions" emit-value map-options dense outlined stack-label :label="labels.clearAfter" name="clearAt"></q-select>' +
                    '</q-card-section>' +
                    '<q-card-actions align="right">' +
                        '<q-btn flat no-caps :label="labels.cancel" v-close-popup></q-btn>' +
                        '<q-btn unelevated no-caps color="primary" :label="labels.save" :loading="saving" id="m-status-save" @click="save"></q-btn>' +
                    '</q-card-actions>' +
                '</q-card>' +
            '</q-dialog>'
    };

    if (typeof module !== "undefined" && module.exports) module.exports = chat;
})(globalThis);
