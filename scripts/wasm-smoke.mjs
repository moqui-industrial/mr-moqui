// Node smoke test for the browser module (plans/Wasm.md).
//   CARGO_TARGET_DIR=target/browser scripts/wasm-build.sh && node scripts/wasm-smoke.mjs target/browser/wasm-pkg/debug
// Boots the factory from the preloaded conf, creates the framework tables (a render reads
// ArtifactAuthz), renders a text screen, loads seed data, calls a service, and does entity CRUD on
// SQLite in MEMFS.
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const pkg = path.resolve(process.argv[2] || 'target/wasm-pkg/debug');
const { default: createMoqui } = await import(pathToFileURL(path.join(pkg, 'moqui_browser.mjs')).href);
const { loadMoqui } = await import(pathToFileURL(path.join(pkg, 'moqui.js')).href);

let failed = false;
function check(label, response, test = () => true) {
    const pass = response.ok && test(response.result);
    if (!pass) failed = true;
    const detail = response.ok ? JSON.stringify(response.result).slice(0, 160) : response.error;
    console.log(`${pass ? 'ok  ' : 'FAIL'} ${label}: ${detail}`);
    return response.result;
}

const started = performance.now();
// Node resolves the preload package against the cwd, not the module; point it at the package.
const moqui = await loadMoqui(createMoqui, { locateFile: (name) => path.join(pkg, name), printErr: () => {} });
const timed = async (label, promise, test) => {
    const at = performance.now();
    const response = await promise;
    check(`${label} (${Math.round(performance.now() - at)} ms)`, response, test);
    return response.result;
};
console.log(`module loaded in ${Math.round(performance.now() - started)} ms`);

await timed('init', moqui.init({ runtime: '/moqui/runtime', framework: '/moqui/framework', db: '/moqui/local.sqlite' }));
await timed('checkTables (all)', moqui.request({ op: 'checkTables' }));
await timed('render text screen', moqui.render({ root: '/moqui/runtime/screen/Hello.xml', path: '', parameters: { name: 'node' } }),
    (result) => result.body.includes('Hello node from Moqui in WASM'));
await timed('loadData SecurityTypeData', moqui.request({ op: 'loadData', location: '/moqui/framework/data/SecurityTypeData.xml', types: ['seed'] }),
    (records) => records > 0);
await timed('callService find#Enumeration', moqui.callService({ name: 'org.moqui.impl.BasicServices.find#Enumeration', parameters: { enumTypeId: 'UserGroupType' } }),
    (out) => out.enumerationList.length > 0);
await timed('store', moqui.request({ op: 'store', entity: 'moqui.test.TestEntity', fields: { testId: 'W1', testMedium: 'from wasm' } }));
await timed('find', moqui.find({ entity: 'moqui.test.TestEntity', equals: { testId: 'W1' } }),
    (rows) => rows.length === 1 && rows[0].testMedium === 'from wasm');
await timed('update', moqui.request({ op: 'store', entity: 'moqui.test.TestEntity', fields: { testId: 'W1', testMedium: 'changed' } }));
await timed('find after update', moqui.find({ entity: 'moqui.test.TestEntity', equals: { testId: 'W1' } }),
    (rows) => rows[0].testMedium === 'changed');
const size = moqui.module.FS.stat('/moqui/local.sqlite').size;
console.log(`database file ${size} bytes in MEMFS`);
console.log(failed ? 'SMOKE FAILED' : 'SMOKE OK');
process.exit(failed ? 1 : 0);
