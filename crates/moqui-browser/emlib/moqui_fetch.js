// emscripten JS library (--js-library, build.rs): the fetch import behind src/fetch.rs.
// moqui_fetch_send is a JSPI import (__async): the WASM stack suspends on the returned Promise.
// The request is copied out of WASM memory before the first await; the response waits here until
// moqui_fetch_take copies it in, so no HEAPU8 view is held across a suspension.
// Module.moquiAbortFetch() aborts the fetch in flight (the worker's `cancel` op).

addToLibrary({
    $moquiFetch: { waiting: new Uint8Array(0), controller: null },

    moqui_fetch_send__deps: ['$moquiFetch', '$UTF8ToString'],
    moqui_fetch_send__async: true,
    moqui_fetch_send: async function (ptr, len) {
        const request = JSON.parse(UTF8ToString(ptr, len));
        const encoder = new TextEncoder();
        const controller = new AbortController();
        moquiFetch.controller = controller;
        Module.moquiAbortFetch = (reason) => controller.abort(reason || new Error('cancelled'));
        const timer = setTimeout(() => controller.abort(new Error('timed out after 30 s')), 30000);
        try {
            const response = await fetch(request.url, {
                method: request.method,
                headers: request.headers,
                body: request.body ?? undefined,
                signal: controller.signal,
                // The browser follows redirects and drops Authorization across origins.
            });
            const bytes = new Uint8Array(await response.arrayBuffer());
            if (bytes.length > request.maxBytes) {
                moquiFetch.waiting = encoder.encode('response is larger than the limit');
                return -1;
            }
            moquiFetch.waiting = bytes;
            return response.status;
        } catch (err) {
            const reason = controller.signal.aborted ? controller.signal.reason : err;
            moquiFetch.waiting = encoder.encode(String(reason && reason.message || reason));
            return -1;
        } finally {
            clearTimeout(timer);
            moquiFetch.controller = null;
            Module.moquiAbortFetch = () => {};
        }
    },

    moqui_fetch_len__deps: ['$moquiFetch'],
    moqui_fetch_len: () => moquiFetch.waiting.length,

    moqui_fetch_take__deps: ['$moquiFetch'],
    moqui_fetch_take: (ptr) => {
        HEAPU8.set(moquiFetch.waiting, ptr);
        moquiFetch.waiting = new Uint8Array(0);
    },
});
