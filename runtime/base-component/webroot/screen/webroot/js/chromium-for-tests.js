/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
// The headless Chromium of the *-browser.test.js files, found as the browser tests of
// moqui test-client find theirs: CHROMIUM_PATH, else MOQUI_CHROMIUM, else runtime/chromium (./moq chromium-download), else
// chromium, chromium-browser or google-chrome on the PATH. Null when there is none, and the test is skipped.
const fs = require('node:fs');
const path = require('node:path');

const isFile = (file) => { try { return fs.statSync(file).isFile(); } catch (err) { return false; } };

const chromium = () => {
    const configured = process.env.CHROMIUM_PATH || process.env.MOQUI_CHROMIUM;
    if (configured) return configured;
    const downloaded = path.join(__dirname, '../../../../../chromium/chrome');
    if (isFile(downloaded)) return downloaded;
    const dirs = (process.env.PATH || '').split(path.delimiter).filter(Boolean);
    for (const name of ['chromium', 'chromium-browser', 'google-chrome']) {
        const found = dirs.map((dir) => path.join(dir, name)).find(isFile);
        if (found) return found;
    }
    return null;
};

const browser = chromium();
// The options of a test that needs the browser: skipped, saying why, when there is none.
const needsBrowser = { skip: browser ? false : 'no Chromium: set CHROMIUM_PATH or MOQUI_CHROMIUM, or run ./moq chromium-download' };

module.exports = { browser, needsBrowser };
