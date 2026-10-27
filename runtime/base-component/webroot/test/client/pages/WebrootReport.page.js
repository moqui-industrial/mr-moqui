/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
"use strict";
const report = moqui.report;
test("columnar frames retain ragged nulls and CSV protects spreadsheet formulas", () => {
    const frame = { fields: [{ name: "name", type: "string", values: ['=SUM(A1)', 'a,"b'] },
        { name: "amount", type: "number", values: [-2] }] };
    assert.deepEqual(report.rows(frame), [['=SUM(A1)', -2], ['a,"b', null]]);
    assert.equal(report.csv(frame), '"name","amount"\r\n"\'=SUM(A1)","-2"\r\n"a,""b",""');
    assert.equal(report.csv({fields:[{name:'numeric',type:'number',values:['-0.3']},{name:'text',type:'string',values:['-2+CMD()']}]}), '"numeric","text"\r\n"-0.3","\'-2+CMD()"');
    assert.equal(report.format(null, {}), "—");
    assert.equal(report.format(null, { noValue: "Missing" }), "Missing");
});
test("chart configs use the frame and retain missing intervals", () => {
    const config = report.chart({ fields: [
        { name: "day", type: "string", values: ["Mon", "Tue", "Wed"] },
        { name: "total", type: "number", values: [1, null, 3] },
    ] }, { visualization: "area", stacked: true });
    assert.equal(config.type, "line");
    assert.deepEqual(config.data.labels, ["Mon", "Tue", "Wed"]);
    assert.deepEqual(config.data.datasets[0].data, [1, null, 3]);
    assert.equal(config.data.datasets[0].spanGaps, false);
    assert.equal(config.data.datasets[0].fill, true);
    assert.equal(config.options.animation, false);
});
test("waterfalls preserve nanosecond order and identify missing parents", () => {
    const spans = [
        { span_id: "child", parent_span_id: "root", name: "service", start_nano: "1800000000000000010", duration_nano: 10 },
        { span_id: "root", name: "request", start_nano: "1800000000000000000", duration_nano: 100 },
        { span_id: "orphan", parent_span_id: "missing", name: "async", start_nano: "1800000000000000050", duration_nano: 10 },
    ];
    const rows = report.waterfall(spans);
    assert.equal(rows[0].span.span_id, "root");
    assert.equal(rows[1].depth, 1);
    assert.equal(rows[1].left, 10);
    assert.equal(rows[1].width, 10);
    assert.equal(rows[2].missingParent, true);
    assert.deepEqual(report.waterfall([]), []);
});
test("malformed cyclic ancestry is bounded", () => {
    const rows = report.waterfall([{ span_id: "a", parent_span_id: "b", start_nano: 0, duration_nano: 1 },
        { span_id: "b", parent_span_id: "a", start_nano: 0, duration_nano: 1 }]);
    assert.equal(rows[0].depth, 1);
    assert.equal(rows[1].depth, 1);
});
test("report widgets render with Vue runtime and escape document values", () => {
    const target = document.createElement("div"); document.body.appendChild(target);
    const app = Vue.createApp({ render() { return Vue.h(moqui.qvtComponents["m-report-panel"], {
        panel: { title: "<img src=x onerror=alert(1)>", visualization: "table" },
        frame: { fields: [{ name: "name", type: "string", values: ["<script>bad()</script>"] }] },
    }); } });
    app.mount(target);
    assert.equal(target.querySelector("img"), null);
    assert.equal(target.querySelector("script"), null);
    assert.equal(target.textContent.includes("<script>bad()</script>"), true);
    app.unmount(); target.remove();
});
test("data links reject executable and protocol-relative URLs", () => {
    assert.equal(report.safeLink("javascript:alert(1)"), "");
    assert.equal(report.safeLink("//foreign.example/path"), "");
    assert.equal(report.safeLink("/apps/monitor/Traces"), "/apps/monitor/Traces");
});
test("field overrides, thresholds and links preserve typed values", () => {
    const fields = report.configuredFields({ fields: [{ name: "latency", type: "number", values: [2] }] }, {
        fieldConfig: { defaults: { decimals: 1 }, overrides: [{ match: { name: "latency" }, config: { unit: "s", thresholds: [{ value: 1, color: "negative" }] } }] },
    });
    assert.equal(fields[0].unit, "s");
    assert.equal(fields[0].decimals, 1);
    assert.equal(report.thresholdColor(2, fields[0]), "var(--q-negative)");
    assert.equal(report.dataLink("/apps/orders?id=${value}&status=${status}", "A&B", { status: "On hold" }), "/apps/orders?id=A%26B&status=On%20hold");
    assert.equal(report.thresholdColor(null, fields[0]), "");
    assert.equal(report.format("2026-01-02T03:04:05Z", { type: "time" }).includes("Invalid"), false);
});
test("sparklines retain gaps and bounded log views deduplicate entries", () => {
    const path = report.sparkline([1, null, 3]);
    assert.equal((path.match(/M/g) || []).length, 2);
    const first = { documentId: "otel_logs", id: "a", body: { body: "first" } };
    const second = { documentId: "otel_logs", id: "b", body: { body: "second" } };
    const third = { documentId: "otel_logs", id: "c", body: { body: "third" } };
    const merged = report.mergeLogHits([first, second], [second, third], 2);
    assert.deepEqual(merged.hits.map((hit) => hit.id), ["b", "c"]);
    assert.equal(merged.removed, 1);
});
test("dashboard tabs and 24-column layouts render the selected panel", async () => {
    const target = document.createElement("div"); document.body.appendChild(target);
    const dashboard = { panels: { first: { title: "First panel", visualization: "stat" }, second: { title: "Second panel", visualization: "stat" } },
        layout: { type: "tabs", items: [{ title: "One", layout: { type: "grid", items: [{ panelId: "first", x: 0, w: 12 }] } },
            { title: "Two", layout: { type: "autoGrid", items: [{ panelId: "second" }] } }] } };
    const frame = { fields: [{ name: "value", type: "number", values: [12] }] };
    const app = Vue.createApp({ render() { return Vue.h(moqui.qvtComponents["m-dashboard"], { dashboard, frames: { first: [frame], second: [frame] } }); } });
    app.mount(target);
    assert.equal(target.textContent.includes("First panel"), true);
    assert.equal(target.textContent.includes("Second panel"), false);
    assert.equal(target.querySelector('[role="tabpanel"] > div').style.gridTemplateColumns.includes("24"), true);
    target.querySelectorAll('[role="tab"]')[1].click(); await Vue.nextTick();
    assert.equal(target.textContent.includes("Second panel"), true);
    assert.equal(target.textContent.includes("First panel"), false);
    app.unmount(); target.remove();
});
test("panel option edits and unlinking a library reference update the draft until save", async () => {
    const target = document.createElement("div"); document.body.appendChild(target);
    const form = { fields: Vue.reactive({ definitionJson: JSON.stringify({ panels: { first: { title: "Original", visualization: "table", queries: [{ datasetId: "otel_logs", mode: "documents" }] } } }) }) };
    const library = { id: "library-test", title: "Shared panel", body: { title: "Shared panel", visualization: "stat", queries: [] } };
    const app = Vue.createApp({ provide() { return { mForm: form }; }, render() { return Vue.h(moqui.qvtComponents["m-report-definition-editor"], { libraries: [library], datasets: [{ id: "otel_logs" }] }); } });
    app.mount(target);
    const title = Array.from(target.querySelectorAll("label")).find((label) => label.textContent === "Panel title").querySelector("input");
    title.value = "Edited title"; title.dispatchEvent(new Event("change")); await Vue.nextTick();
    assert.equal(JSON.parse(form.fields.definitionJson).panels.first.title, "Edited title");
    Array.from(target.querySelectorAll('button')).find(button=>button.textContent==='Add aggregation').click();await Vue.nextTick();
    assert.equal(Array.isArray(JSON.parse(form.fields.definitionJson).panels.first.queries[0].aggregations),true);
    assert.equal(JSON.parse(form.fields.definitionJson).panels.first.queries[0].mode,'aggregations');
    Array.from(target.querySelectorAll('button')).find(button=>button.textContent==='Add transformation').click();await Vue.nextTick();
    assert.equal(JSON.parse(form.fields.definitionJson).panels.first.transformations[0].count,100);
    const layout = Array.from(target.querySelectorAll("label")).find((label) => label.textContent.startsWith("Layout ")).querySelector("select");
    layout.value = "grid"; layout.dispatchEvent(new Event("change")); await Vue.nextTick();
    assert.equal(JSON.parse(form.fields.definitionJson).layout.type, "grid");
    const pick = Array.from(target.querySelectorAll("label")).find((label) => label.textContent.startsWith("Library panel ")).querySelector("select");
    pick.value = library.id; pick.dispatchEvent(new Event("change")); await Vue.nextTick();
    Array.from(target.querySelectorAll("button")).find((button) => button.textContent === "Add library reference").click(); await Vue.nextTick();
    const body = JSON.parse(form.fields.definitionJson), id = Object.keys(body.panels).find((id) => body.panels[id].panelRef === library.id);
    assert.equal(!!id, true);
    Array.from(target.querySelectorAll("button")).find((button) => button.textContent === "Unlink as copy").click(); await Vue.nextTick();
    const unlinked = JSON.parse(form.fields.definitionJson).panels[id];
    assert.equal(unlinked.panelRef, undefined);
    assert.equal(unlinked.title, "Shared panel");
    assert.equal(library.body.title, "Shared panel");
    app.unmount(); target.remove();
});
test("annotations map to chart positions while out-of-range markers stay outside the view", () => {
    const frame = { fields: [{ name: "time", type: "time", values: [1000, 2000, 3000] }, { name: "requests", type: "number", values: [1, 2, 3] }] };
    const annotations = [{ time: 2000, time_end: 3000, text: "Deployment" }, { time: 500, text: "Earlier" }];
    const markers = report.annotationPositions(frame, annotations);
    assert.equal(markers.length, 1);
    assert.equal(markers[0].start, 1);
    assert.equal(markers[0].end, 2);
    assert.equal(report.chart(frame, { annotations }).plugins[0].id, "moquiAnnotations");
});
test("panel export keeps exact range and variables and decodes server XLSX bytes", async () => {
    const target=document.createElement('div');document.body.appendChild(target);
    const previousFetch=window.fetch,previousCreate=URL.createObjectURL,previousRevoke=URL.revokeObjectURL,previousClick=HTMLAnchorElement.prototype.click;
    let request,blob,download;
    window.fetch=async url=>{request=String(url);return {ok:true,json:async()=>({contentType:'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',filename:'latency.xlsx',contentBase64:btoa('PK\x03\x04')})};};
    URL.createObjectURL=value=>{blob=value;return 'blob:report-test';};URL.revokeObjectURL=()=>{};
    HTMLAnchorElement.prototype.click=function(){download=this.download;};
    const app=Vue.createApp(moqui.qvtComponents['m-report-panel'],{dashboardId:'dashboard',panelId:'latency',range:{from:'2026-01-01T00:00:00Z',thru:'2026-01-02T00:00:00Z'},variables:{service:'ERP'}});
    try {
        const panel=app.mount(target);await panel.exportPanel('xlsx');
        const query=new URL(request,window.location.href).searchParams;
        assert.equal(query.get('from'),'2026-01-01T00:00:00Z');assert.equal(query.get('panelId'),'latency');
        assert.deepEqual(JSON.parse(query.get('variables')),{service:'ERP'});
        assert.deepEqual(Array.from(new Uint8Array(await blob.arrayBuffer())),[80,75,3,4]);assert.equal(download,'latency.xlsx');
    } finally {app.unmount();target.remove();window.fetch=previousFetch;URL.createObjectURL=previousCreate;URL.revokeObjectURL=previousRevoke;HTMLAnchorElement.prototype.click=previousClick;}
});
test("playlist controls move between accessible dashboards and pause rotation", async () => {
    const target=document.createElement('div');document.body.appendChild(target);
    const app=Vue.createApp(moqui.qvtComponents['m-report-playlist'],{items:[{title:'First',url:'/first'},{title:'Second',url:'/second'}],seconds:3600});
    const playlist=app.mount(target);
    assert.equal(target.querySelector('iframe').title,'First');
    Array.from(target.querySelectorAll('button')).find(button=>button.textContent==='Next').click();await Vue.nextTick();
    assert.equal(target.querySelector('iframe').title,'Second');
    Array.from(target.querySelectorAll('button')).find(button=>button.textContent==='Pause').click();await Vue.nextTick();
    assert.equal(playlist.paused,true);
    app.unmount();target.remove();
});
test("structural pills retain typed values, negation, disable, labels and user-scoped pins", async () => {
    const target=document.createElement('div');document.body.appendChild(target);
    const form={fields:Vue.reactive({queryString:'',filtersJson:'[]'})},props={datasetId:'pill-test',viewerId:'reader-test',fields:[{name:'duration',type:'number',searchable:true},{name:'status',type:'string',searchable:true}]};
    localStorage.removeItem(report.storageKey('reader-test','pill-test')+'.pins');
    const app=Vue.createApp(moqui.qvtComponents['m-report-querybar'],props);app.provide('mForm',form);const editor=app.mount(target);
    editor.field='duration';editor.operation='range';editor.value='1,5';editor.label='Expected duration';editor.add();await Vue.nextTick();
    let pill=JSON.parse(form.fields.filtersJson)[0];assert.deepEqual(pill.query,{field:{name:'duration',op:'range',gte:1,lte:5}});
    editor.change(0,'pinned');editor.change(0,'negated');editor.change(0,'disabled');await Vue.nextTick();
    pill=JSON.parse(form.fields.filtersJson)[0];assert.equal(pill.pinned,true);assert.equal(pill.negated,true);assert.equal(pill.disabled,true);
    assert.equal(target.textContent.includes('NOT Expected duration (disabled)'),true);
    editor.edit(0);editor.editJson=JSON.stringify({field:{name:'status',op:'eq',value:'ERROR'}});editor.label='Failures';editor.saveEdit();
    assert.equal(JSON.parse(form.fields.filtersJson)[0].label,'Failures');
    app.unmount();target.remove();
    const next=document.createElement('div');document.body.appendChild(next);const nextForm={fields:Vue.reactive({queryString:'',filtersJson:'[]'})};
    const nextApp=Vue.createApp(moqui.qvtComponents['m-report-querybar'],props);nextApp.provide('mForm',nextForm);const restored=nextApp.mount(next);
    assert.equal(JSON.parse(nextForm.fields.filtersJson)[0].label,'Failures');restored.remove(0);assert.deepEqual(JSON.parse(nextForm.fields.filtersJson),[]);
    nextApp.unmount();next.remove();localStorage.removeItem(report.storageKey('reader-test','pill-test')+'.pins');
});
test("query history and autocomplete reuse authorized fields and sampled values", async () => {
    assert.deepEqual(report.suggestions('severity_text:E',[{name:'severity_text',type:'string'}],{severity_text:['ERROR','WARN']}).map(item=>item.value),['severity_text:"ERROR"']);
    assert.equal(report.replaceQueryToken('name:ERP AND severity_text:E','severity_text:"ERROR"'),'name:ERP AND severity_text:"ERROR"');
    const target=document.createElement('div');document.body.appendChild(target);const form={fields:Vue.reactive({queryString:'severity_text:ERROR',filtersJson:'[]'})};
    const app=Vue.createApp(moqui.qvtComponents['m-report-querybar'],{datasetId:'history-test',viewerId:'history-reader',fields:[{name:'severity_text',type:'string'}]});app.provide('mForm',form);const editor=app.mount(target);
    assert.equal(editor.history[0],'severity_text:ERROR');editor.query='sev';editor.suggesting=true;await Vue.nextTick();
    const suggestion=target.querySelector('[role="listbox"] button');assert.equal(suggestion.textContent.includes('severity_text'),true);suggestion.click();await Vue.nextTick();
    assert.equal(form.fields.queryString,'severity_text:');editor.query='name:ERP';editor.remember();assert.equal(editor.history[0],'name:ERP');
    assert.deepEqual(report.storage.read(report.storageKey('other-user','history-test')+'.history',[]),[]);
    app.unmount();target.remove();localStorage.removeItem(report.storageKey('history-reader','history-test')+'.history');
});
test("real Chart.js panels share timestamps across different bucket widths and synchronize brushed ranges", () => {
    const frame=times=>({fields:[{name:'time',type:'time',values:times},{name:'requests',type:'number',values:times.map(()=>1)}]});
    const first=document.createElement('canvas'),second=document.createElement('canvas');document.body.append(first,second);
    const a=new Chart(first,report.chart(frame([1000,2000,3000]),{syncGroup:'chart-test'})),b=new Chart(second,report.chart(frame([1000,1500,2000,2500,3000]),{syncGroup:'chart-test'}));
    const plugin=a.config.plugins.find(plugin=>plugin.id==='moquiReportSync'),range=[];const onRange=event=>range.push(event.detail);document.addEventListener('moqui-report-range',onRange);
    try {
        plugin.afterEvent(a,{event:{type:'mousemove',x:a.scales.x.getPixelForValue(1),y:a.chartArea.top+5}});
        assert.equal(b.getActiveElements()[0].index,2);
        plugin.afterEvent(a,{event:{type:'mousedown',x:a.scales.x.getPixelForValue(0),y:a.chartArea.top+5}});
        plugin.afterEvent(a,{event:{type:'mouseup',x:a.scales.x.getPixelForValue(1),y:a.chartArea.top+5}});
        assert.equal(a.options.scales.x.max,1);assert.equal(b.options.scales.x.max,2);assert.equal(range[0].from,'1970-01-01T00:00:01.000Z');assert.equal(range[0].thru,'1970-01-01T00:00:02.000Z');
        report.chartSync.reset('chart-test');assert.equal(a.options.scales.x.max,undefined);assert.equal(b.options.scales.x.max,undefined);
        plugin.afterEvent(a,{event:{type:'mouseout'}});assert.equal(b.getActiveElements().length,0);
    } finally {document.removeEventListener('moqui-report-range',onRange);a.destroy();b.destroy();first.remove();second.remove();}
});
test("dashboard keyboard prefixes shift exact ranges and inspect the selected panel", async () => {
    assert.deepEqual(report.shiftRange({from:'2026-01-01T00:00:00Z',thru:'2026-01-02T00:00:00Z'},'ArrowLeft'),{from:'2025-12-31T00:00:00.000Z',thru:'2026-01-01T00:00:00.000Z'});
    assert.deepEqual(report.shiftRange({from:'2026-01-01T00:00:00Z',thru:'2026-01-02T00:00:00Z'},'-'),{from:'2025-12-31T12:00:00.000Z',thru:'2026-01-02T12:00:00.000Z'});
    const target=document.createElement('div');document.body.appendChild(target);
    const app=Vue.createApp(moqui.qvtComponents['m-dashboard'],{dashboard:{panels:{first:{title:'Keyboard panel',visualization:'table'}}},frames:{first:[{fields:[{name:'keyboard-value',type:'number',values:[7]}]}]}});
    const dashboard=app.mount(target);await Vue.nextTick();
    dashboard.onKey({key:'i',target:document.body,preventDefault(){}});await Vue.nextTick();
    assert.equal(target.querySelector('pre').textContent.includes('keyboard-value'),true);
    dashboard.onKey({key:'d',target:document.body,preventDefault(){}});dashboard.onKey({key:'k',target:document.body,preventDefault(){}});await Vue.nextTick();
    assert.equal(dashboard.kiosk,true);
    app.unmount();target.remove();
});
test("retained partial buckets use the typed time axis and exclude metadata series", () => {
    const frame={fields:[{name:'partial',type:'boolean',values:[true,false]},{name:'__level',type:'number',values:[0,0]},{name:'__subtotal',type:'number',values:[1,0]},{name:'time',type:'time',values:[1000,2000]},{name:'requests',type:'number',values:[3,4]}]};
    const config=report.chart(frame,{visualization:'timeSeries'});
    assert.deepEqual(config.data.labels,[report.format(1000,{type:'time'}),report.format(2000,{type:'time'})]);
    assert.deepEqual(config.data.datasets.map(dataset=>dataset.label),['requests']);
    assert.deepEqual(config.data.datasets[0].data,[3,4]);
    const textAxis=report.chart({fields:[frame.fields[0],{name:'category',type:'string',values:['A','B']},frame.fields[4]]},{});
    assert.deepEqual(textAxis.data.labels,['A','B']);
});
