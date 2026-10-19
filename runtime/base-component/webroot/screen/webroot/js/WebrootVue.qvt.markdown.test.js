/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// Tests of m-markdown's rendering (moqui.markdown in WebrootVue.qvt.js) on the vendored Marked, with no browser: wiki links,
// heading ids and the outline, a first title left out, links through a resolver or from a base path, external links in a new tab
// with noopener, anchors, tables in their container, raw HTML left to DOMPurify (whose options are checked here; DOMPurify needs a
// DOM, and the cleaning itself is the Chromium smoke of Gate 3, crates/moqui-host/tests/gate3_browser.rs).
// Run: node --test runtime/base-component/webroot/screen/webroot/js/WebrootVue.qvt.markdown.test.js
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");

globalThis.moqui = require("./MoquiLib.js");
require("./WebrootVue.qvt.js");
const marked = require("../libs/marked/marked.umd.min.js");
const md = moqui.markdown;

test("[[path]] and [[path|text]] are wiki: links outside fenced code and code spans", () => {
    assert.equal(md.expandWikiLinks("See [[Policies/Travel]] and [[Home|the start]]."),
        "See [Policies/Travel](wiki:Policies/Travel) and [the start](wiki:Home).");
    assert.equal(md.expandWikiLinks("a `[[not]]` b"), "a `[[not]]` b");
    assert.equal(md.expandWikiLinks("```\n[[in code]]\n```\n[[out]]"), "```\n[[in code]]\n```\n[out](wiki:out)");
    assert.equal(md.expandWikiLinks("[[Two Words/And more]]"), "[Two Words/And more](wiki:Two%20Words/And%20more)");
});

test("a heading's id is its words, made unique, after the prefix; the outline lists the headings", () => {
    assert.equal(md.slug("Hello, World!"), "hello-world");
    assert.equal(md.slug("Größe & Café"), "größe-café");
    assert.equal(md.slug("***"), "section");
    const out = md.render(marked, "# Intro\n\n## Setup *now*\n\n## Setup now\n\n### A &amp; B", {});
    assert.match(out.html, /<h1 id="md-intro">Intro<\/h1>/);
    assert.match(out.html, /<h2 id="md-setup-now">Setup <em>now<\/em><\/h2>/);
    assert.match(out.html, /<h2 id="md-setup-now-1">Setup now<\/h2>/);
    assert.deepEqual(out.outline, [
        { id: "md-intro", level: 1, text: "Intro" }, { id: "md-setup-now", level: 2, text: "Setup now" },
        { id: "md-setup-now-1", level: 2, text: "Setup now" }, { id: "md-a-b", level: 3, text: "A & B" }
    ]);
    const prefixed = md.render(marked, "## One", { headingPrefix: "k-" });
    assert.match(prefixed.html, /<h2 id="k-one">/);
});

test("a first heading that is the screen's title is left out", () => {
    const out = md.render(marked, "# Travel **Policy**\n\nText\n\n# Travel Policy", { stripTitle: "Travel Policy" });
    assert.equal(out.html.indexOf("<h1"), out.html.lastIndexOf("<h1"), out.html);
    assert.match(out.html, /^<p>Text<\/p>/);
    assert.equal(out.outline.length, 1);
    assert.match(md.render(marked, "Intro\n\n# Travel Policy", { stripTitle: "Travel Policy" }).html, /<h1/, "only a first heading");
});

test("links: external in a new tab with noopener, an anchor to its heading, a relative one from the base path, a path through the shell", () => {
    const out = md.render(marked, "[site](https://example.com \"Ex\") [mail](mailto:a@b.c) [top](#Intro) [doc](guide/Start?x=1) [up](../Other) [abs](/apps/x)",
        { basePath: "/apps/wiki/Space" });
    assert.match(out.html, /<a href="https:\/\/example.com" title="Ex" target="_blank" rel="noopener noreferrer">site<\/a>/);
    assert.match(out.html, /<a href="mailto:a@b.c">mail<\/a>/);
    assert.match(out.html, /<a href="#Intro" data-md-anchor="md-intro">top<\/a>/);
    assert.match(out.html, /<a href="\/apps\/wiki\/Space\/guide\/Start\?x=1" data-md-link>doc<\/a>/);
    assert.match(out.html, /<a href="\/apps\/wiki\/Other" data-md-link>up<\/a>/);
    assert.match(out.html, /<a href="\/apps\/x" data-md-link>abs<\/a>/);
    assert.equal(md.linkKind("javascript:alert(1)"), "external", "another scheme is never a shell path (DOMPurify drops it)");
    assert.equal(md.linkKind("//cdn.example.com/x"), "external");
    assert.equal(md.linkKind("wiki:Home"), "wiki");
    assert.equal(md.resolvePath("../../x", "/a"), "/x");
});

test("a resolver decides a link: a new href, a missing page in the negative color, or the default", () => {
    const seen = [];
    const resolveLink = (href, info) => {
        seen.push(href + ":" + info.kind + (info.image ? ":image" : ""));
        if (href.startsWith("wiki:")) return { href: "/qapps/commons/Knowledge?pagePath=" + encodeURIComponent(href.slice(5)), missing: href === "wiki:Gone", title: "Page" };
        if (href === "pic.png") return "/files/pic.png";
        return null;
    };
    const out = md.render(marked, "[[Home]] [[Gone]] [x](https://x.org) ![a picture](pic.png)", { wikiLinks: true, resolveLink, basePath: "/b" });
    assert.match(out.html, /<a href="\/qapps\/commons\/Knowledge\?pagePath=Home" title="Page" data-md-link>Home<\/a>/);
    assert.match(out.html, /<a href="\/qapps\/commons\/Knowledge\?pagePath=Gone" title="Page" class="text-negative" data-md-link>Gone<\/a>/);
    assert.match(out.html, /<a href="https:\/\/x.org" target="_blank" rel="noopener noreferrer">x<\/a>/, "null keeps the default");
    assert.match(out.html, /<img src="\/files\/pic.png" alt="a picture">/);
    assert.deepEqual(seen, ["wiki:Home:wiki", "wiki:Gone:wiki", "https://x.org:external", "pic.png:relative:image"]);
    // a resolver that throws leaves the link as it is
    const warn = console.warn;
    const warned = [];
    console.warn = (...args) => warned.push(args.join(" "));
    let safe;
    try { safe = md.render(marked, "[a](b)", { resolveLink: () => { throw new Error("no"); }, basePath: "/p" }); } finally { console.warn = warn; }
    assert.equal(warned.length, 1);
    assert.match(safe.html, /<a href="\/p\/b" data-md-link>a<\/a>/);
});

test("a table is in a scrolling q-table container, code keeps its language, and raw HTML waits for DOMPurify", () => {
    const out = md.render(marked, "| a | b |\n|---|---|\n| 1 | 2 |\n\n```js\nlet x = 1 < 2\n```\n\n<script>alert(1)</script>", {});
    assert.match(out.html, /<div class="m-md-table q-table__container q-table--flat q-table--bordered q-table--dense"><table class="q-table">/);
    assert.match(out.html, /<\/table><\/div>/);
    assert.match(out.html, /<pre><code class="language-js">let x = 1 &lt; 2/);
    let options = null;
    const cleaned = md.sanitize({ sanitize: (html, given) => { options = given; return html.replace(/<script>.*<\/script>/, ""); } }, out.html);
    assert.ok(cleaned.indexOf("<script") < 0);
    assert.deepEqual(options.ADD_ATTR, ["target"]);
    assert.ok(options.FORBID_TAGS.includes("style") && options.FORBID_TAGS.includes("form"));
    assert.deepEqual(options.FORBID_ATTR, ["style"]);
    assert.deepEqual(md.render(marked, "", {}), { html: "", outline: [] });
});

test("the markdown component's template compiles and takes its props", () => {
    const comp = moqui.qvt.components["m-markdown"];
    assert.ok(comp);
    ["text", "basePath", "resolveLink", "headingPrefix", "stripTitle", "wikiLinks"].forEach((name) => assert.ok(Object.keys(comp.props).includes(name), name));
    assert.deepEqual(comp.emits, ["outline", "rendered", "link"]);
});
