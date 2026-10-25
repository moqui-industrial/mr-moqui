/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
// The headless Chromium of the browser tests, found as PDF and moqui test-client find theirs (crates/moqui-host/src/chromium.rs):
// the configured one (CHROMIUM_PATH, MOQUI_CHROMIUM or pdf_chromium_path) when it names a file, else the one of
// ./moq chromium-download in runtime/chromium, else chromium or chromium-browser on the PATH, else Google Chrome
// (google-chrome, google-chrome-stable), and on macOS those in /Applications. Null when there is none, and the test is
// skipped; a configured path that is not a file is an error.
const fs = require('node:fs');
const path = require('node:path');

const isFile = (file) => { try { return fs.statSync(file).isFile(); } catch (err) { return false; } };

const chromium = () => {
    for (const name of ['CHROMIUM_PATH', 'MOQUI_CHROMIUM', 'pdf_chromium_path']) {
        const configured = (process.env[name] || '').trim();
        if (!configured) continue;
        if (!isFile(configured)) throw new Error(`${name} names ${configured}, which is not a file`);
        return configured;
    }
    const runtime = path.join(__dirname, '../../../../..');
    const candidates = ['chrome', 'Chromium.app/Contents/MacOS/Chromium'].map((name) => path.join(runtime, 'chromium', name));
    const dirs = (process.env.PATH || '').split(path.delimiter).filter(Boolean);
    const onPath = (name) => dirs.map((dir) => path.join(dir, name)).find(isFile);
    candidates.push(onPath('chromium'), onPath('chromium-browser'));
    if (process.platform === 'darwin') candidates.push('/Applications/Chromium.app/Contents/MacOS/Chromium');
    candidates.push(onPath('google-chrome'), onPath('google-chrome-stable'));
    if (process.platform === 'darwin') candidates.push('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome');
    return candidates.find((file) => file && isFile(file)) || null;
};

const browser = chromium();
// The options of a test that needs the browser: skipped, saying why, when there is none.
const needsBrowser = {
    skip: browser ? false : 'no Chromium: set pdf_chromium_path (or CHROMIUM_PATH), run ./moq chromium-download, or put chromium or google-chrome on the PATH'
};

module.exports = { browser, needsBrowser };
