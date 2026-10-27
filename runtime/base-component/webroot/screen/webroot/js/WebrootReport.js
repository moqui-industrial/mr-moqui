/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
/* Reporting widgets use server-computed frames. Render functions also run on the compiler-free Vue build. */
(function (root) {
    "use strict";
    const moqui = root.moqui;
    const report = moqui.report = {};
    const finite = (v) => v !== null && v !== undefined && v !== "" && Number.isFinite(Number(v));
    report.fields = (frame) => (frame && Array.isArray(frame.fields) ? frame.fields : []);
    report.configuredFields = (frame, panel) => report.fields(frame).map((field) => {
        let result = Object.assign({}, panel && panel.fieldConfig && panel.fieldConfig.defaults || {}, field);
        for (const override of panel && panel.fieldConfig && panel.fieldConfig.overrides || []) {
            const match = override.match || {};
            let matches = match.name ? field.name === match.name : match.type ? field.type === match.type : false;
            if (match.regex && String(match.regex).length <= 256) { try { matches = new RegExp(match.regex).test(field.name); } catch (_) { matches = false; } }
            if (matches) result = Object.assign(result, override.config || {});
        }
        return result;
    });
    report.rows = (frame) => {
        const fields = report.fields(frame);
        const count = fields.reduce((n, field) => Math.max(n, (field.values || []).length), 0);
        return Array.from({ length: count }, (_, at) => fields.map((field) => (field.values || [])[at] ?? null));
    };
    report.format = (value, field) => {
        const config = Object.assign({}, field || {}, field && field.config || {});
        if (value === null || value === undefined) return config.noValue || "—";
        for (const mapping of config.mappings || []) {
            if ((mapping.value !== undefined && String(mapping.value) === String(value)) ||
                    (finite(value) && mapping.from !== undefined && Number(value) >= mapping.from && Number(value) <= mapping.to)) return mapping.text;
        }
        if (config.type === "time" || config.kind === "time") {
            const date = new Date(/^\d+$/.test(String(value)) ? Number(value) : value);
            return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
        }
        if (!finite(value) || (config.type && !["number", "integer"].includes(config.type))) return String(value);
        let n = Number(value), suffix = "";
        if (config.unit === "duration" || config.unit === "ns") { n /= 1e6; suffix = " ms"; }
        else if (config.unit === "s") suffix = " s";
        else if (config.unit === "ms") suffix = " ms";
        else if (config.unit === "percent" || config.unit === "%") suffix = "%";
        else if (config.unit === "bytes") {
            const units = [" B", " KiB", " MiB", " GiB", " TiB"];
            let at = 0;
            while (Math.abs(n) >= 1024 && at < units.length - 1) { n /= 1024; at++; }
            suffix = units[at];
        }
        if (/^(currency:)?[A-Z]{3}$/.test(config.unit || "")) {
            const currency = config.unit.replace(/^currency:/, "");
            try { return n.toLocaleString(undefined, { style: "currency", currency, maximumFractionDigits: config.decimals ?? 2 }); } catch (_) { /* unknown unit remains a number */ }
        }
        const decimals = Math.min(12, Math.max(0, config.decimals ?? (Number.isInteger(n) ? 0 : 2)));
        return n.toLocaleString(undefined, { minimumFractionDigits: decimals, maximumFractionDigits: decimals }) + suffix;
    };
    report.csv = (frame) => {
        // Spreadsheet programs interpret these prefixes even in quoted CSV cells.
        const cell = (value, field) => {
            let text = value === null || value === undefined ? "" : String(value);
            const numeric = field && field.type === "number" && /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(text);
            if (typeof value === "string" && !numeric && /^[=+@\-\t\r]/.test(text)) text = "'" + text;
            return '"' + text.replace(/"/g, '""') + '"';
        };
        const fields = report.fields(frame);
        return [fields.map((field) => cell(field.name)).join(","), ...report.rows(frame).map((row) => row.map((value, at) => cell(value, fields[at])).join(","))].join("\r\n");
    };
    report.color = (at) => {
        const names = ["primary", "positive", "warning", "info", "negative", "secondary", "accent"];
        const name = names[at % names.length];
        return root.getComputedStyle && root.document ? root.getComputedStyle(root.document.documentElement).getPropertyValue("--q-" + name).trim() || name : name;
    };
    report.annotationPositions = (frame, annotations) => {
        const field = report.fields(frame).find((item) => item.type === "time" || ["time", "timestamp", "bucket_start", "start"].includes(item.name));
        if (!field) return [];
        const stamp = (value) => /^\d+$/.test(String(value)) ? Number(value) : Date.parse(value);
        const times = (field.values || []).map(stamp);
        const index = (at) => {
            if (!Number.isFinite(at) || !times.length || at < times[0] || at > times[times.length - 1]) return null;
            let low = 0, high = times.length - 1;
            while (low < high) { const mid = Math.floor((low + high) / 2); if (times[mid] < at) low = mid + 1; else high = mid; }
            return low;
        };
        return (annotations || []).slice(0, 200).map((annotation) => ({ annotation, start: index(stamp(annotation.time)), end: annotation.time_end ? index(stamp(annotation.time_end)) : null }))
            .filter((item) => item.start !== null);
    };
    const syncCharts=new Map();
    report.shiftRange=(range,key)=>{
        const from=Date.parse(range && range.from),thru=Date.parse(range && range.thru),width=thru-from;
        if (!Number.isFinite(width) || width<=0)return null;
        const offsets=key==='-'?[-width/2,width/2]:key==='ArrowLeft'?[-width,-width]:key==='ArrowRight'?[width,width]:null;
        return offsets?{from:new Date(from+offsets[0]).toISOString(),thru:new Date(thru+offsets[1]).toISOString()}:null;
    };
    report.chartSync = {
        group(name){if(!syncCharts.has(name))syncCharts.set(name,new Set());return syncCharts.get(name);},
        clear(name){for(const chart of this.group(name)){chart.$reportCursor=null;chart.setActiveElements([]);if(chart.tooltip)chart.tooltip.setActiveElements([],{x:0,y:0});chart.draw();}},
        zoom(name,from,thru,origin){for(const chart of this.group(name)){const values=chart.$reportValues || [];if(!values.length)continue;const a=report.nearest(values,from),b=report.nearest(values,thru);chart.options.scales.x.min=Math.min(a,b);chart.options.scales.x.max=Math.max(a,b);chart.update('none');}
            if(origin && Number.isFinite(from) && Number.isFinite(thru) && thru>from)root.document.dispatchEvent(new root.CustomEvent('moqui-report-range',{detail:{group:name,from:new Date(from).toISOString(),thru:new Date(thru).toISOString()}}));},
        reset(name){for(const chart of this.group(name)){delete chart.options.scales.x.min;delete chart.options.scales.x.max;chart.update('none');}}
    };
    report.timeValue=value=>typeof value==='number'||/^\d+$/.test(String(value))?Number(value):Date.parse(value);
    report.nearest=(values,value)=>{let best=0,distance=Infinity;values.forEach((item,at)=>{const d=typeof item==='number' && typeof value==='number'?Math.abs(item-value):String(item)===String(value)?0:Infinity;if(d<distance){best=at;distance=d;}});return best;};
    report.syncPlugin=(frame,group)=>{
        const fields=report.fields(frame),field=fields.find(item=>item.type==='time') || fields.find(item=>!report.isMetadata(item) && item.type!=='number') || fields[0];
        const isTime=field && (field.type==='time'||['time','timestamp','bucket_start'].includes(field.name));
        const values=(field && field.values || []).map(value=>isTime?report.timeValue(value):value);
        return {id:'moquiReportSync',afterInit(chart){chart.$reportValues=values;report.chartSync.group(group).add(chart);},afterDestroy(chart){report.chartSync.group(group).delete(chart);if(!report.chartSync.group(group).size)syncCharts.delete(group);},
            afterEvent(chart,args){const event=args.event,scale=chart.scales.x;if(!scale || !values.length)return;const at=Math.max(0,Math.min(values.length-1,Math.round(scale.getValueForPixel(event.x))));
                if(event.type==='mouseout'){report.chartSync.clear(group);return;}
                if(event.type==='mousedown'||event.type==='touchstart'){chart.$reportDrag={at,x:event.x};return;}
                if(event.type==='dblclick'){report.chartSync.reset(group);return;}
                if(event.type==='mouseup'||event.type==='touchend'){const start=chart.$reportDrag;chart.$reportDrag=null;if(isTime && start && Math.abs(start.x-event.x)>4){const from=Math.min(values[start.at],values[at]),thru=Math.max(values[start.at],values[at]);if(thru>from)report.chartSync.zoom(group,from,thru,chart);}return;}
                if(event.type!=='mousemove'&&event.type!=='touchmove')return;
                for(const other of report.chartSync.group(group)){const index=report.nearest(other.$reportValues || [],values[at]),active=other.data.datasets.map((_,datasetIndex)=>({datasetIndex,index}));other.$reportCursor=index;other.setActiveElements(active);if(other.tooltip)other.tooltip.setActiveElements(active,{x:other.scales.x.getPixelForValue(index),y:other.chartArea.top});if(other!==chart)other.draw();}args.changed=true;
            },afterDraw(chart){if(chart.$reportCursor==null||!chart.scales.x||!chart.chartArea)return;const x=chart.scales.x.getPixelForValue(chart.$reportCursor),area=chart.chartArea;if(x<area.left||x>area.right)return;const ctx=chart.ctx;ctx.save();ctx.strokeStyle=report.color(3);ctx.setLineDash([4,3]);ctx.beginPath();ctx.moveTo(x,area.top);ctx.lineTo(x,area.bottom);ctx.stroke();ctx.restore();}
        };
    };

    report.isMetadata=field=>["partial","__subtotal","__level"].includes(field.name);
    report.chart = (frame, panel) => {
        panel = panel || {};
        const fields = report.fields(frame), rows = report.rows(frame);
        const time=fields.findIndex(field=>(field.type || field.kind)==='time');
        const x=time>=0?time:Math.max(0,fields.findIndex(field=>!report.isMetadata(field) && (field.type || field.kind)!=='number'));
        const numeric = fields.map((field, at) => ({ field, at })).filter(({ field }) => !report.isMetadata(field) && (field.type || field.kind) === "number");
        const visual = panel.visualization || panel.type || "timeSeries";
        const type = ["pie", "donut"].includes(visual) ? (visual === "donut" ? "doughnut" : "pie") : visual === "bar" ? "bar" : "line";
        const annotations = report.annotationPositions(frame, panel.annotations);
        const sync=panel.syncGroup && !["pie","donut"].includes(visual)?[report.syncPlugin(frame,panel.syncGroup)]:[];
        return { type, plugins: annotations.length ? [{ id: "moquiAnnotations", afterDraw(chart) {
            const scale = chart.scales.x, area = chart.chartArea, ctx = chart.ctx;
            if (!scale || !area) return;
            ctx.save(); ctx.beginPath(); ctx.rect(area.left, area.top, area.right - area.left, area.bottom - area.top); ctx.clip();
            for (const marker of annotations) {
                const left = scale.getPixelForValue(marker.start), right = marker.end === null ? null : scale.getPixelForValue(marker.end);
                ctx.fillStyle = report.color(2); ctx.strokeStyle = report.color(2);
                if (right !== null) { ctx.globalAlpha = 0.12; ctx.fillRect(left, area.top, Math.max(1, right - left), area.bottom - area.top); }
                ctx.globalAlpha = 0.8; ctx.beginPath(); ctx.moveTo(left, area.top); ctx.lineTo(left, area.bottom); ctx.stroke();
            }
            ctx.restore();
        } },...sync] : sync, data: { labels: rows.map((row) => report.format(row[x], fields[x])),
            datasets: numeric.map(({ field, at }, index) => ({ label: field.displayName || field.name,
                data: rows.map((row) => finite(row[at]) ? Number(row[at]) : null), borderColor: report.color(index),
                backgroundColor: ["pie", "donut"].includes(visual) ? rows.map((_, i) => report.color(i)) : report.color(index),
                fill: visual === "area", spanGaps: false, tension: 0, pointRadius: 1, yAxisID: index > 0 && panel.secondAxis ? "y1" : "y" })) },
            options: { animation: false, responsive: true, maintainAspectRatio: false,
                events:["mousemove","mouseout","mousedown","mouseup","touchstart","touchmove","touchend","dblclick"],
                interaction: { mode: "index", intersect: false },
                scales: ["pie", "donut"].includes(visual) ? undefined : { x: { stacked: !!panel.stacked }, y: { stacked: !!panel.stacked },
                    ...(panel.secondAxis ? { y1: { position: "right", grid: { drawOnChartArea: false } } } : {}) } } };
    };
    report.waterfall = (spans) => {
        spans = (spans || []).map((span) => span.body || span);
        if (!spans.length) return [];
        // Subtract epoch nanoseconds before converting to a plotting coordinate. Decimal strings retain ns precision.
        const ns = (v) => { try { return BigInt(String(v || 0).split(".")[0]); } catch (_) { return 0n; } };
        const start = (span) => span.start_nano !== undefined ? ns(span.start_nano) : ns(span.start) * 1000000n;
        const first = spans.reduce((a, span) => start(span) < a ? start(span) : a, start(spans[0]));
        const last = spans.reduce((a, span) => start(span) + ns(span.duration_nano) > a ? start(span) + ns(span.duration_nano) : a, first);
        const duration = Number(last - first) || 1;
        const byId = new Map(spans.map((span) => [span.span_id, span]));
        const depth = (span) => {
            const seen = new Set([span.span_id]); let n = 0, parent = span.parent_span_id;
            while (parent && byId.has(parent) && !seen.has(parent) && n < 64) { seen.add(parent); n++; parent = byId.get(parent).parent_span_id; }
            return n;
        };
        return spans.slice().sort((a, b) => start(a) < start(b) ? -1 : start(a) > start(b) ? 1 : String(a.span_id).localeCompare(String(b.span_id))).map((span) => ({
            span, depth: depth(span), left: 100 * Number(start(span) - first) / duration,
            width: Math.min(100, Math.max(0.3, 100 * Number(ns(span.duration_nano)) / duration)), duration: Number(ns(span.duration_nano)) / 1e6,
            missingParent: !!span.parent_span_id && !/^0+$/.test(span.parent_span_id) && !byId.has(span.parent_span_id) }));
    };
    report.safeLink = (url) => typeof url === "string" && (url.startsWith("/") && !url.startsWith("//") || /^https?:\/\//i.test(url)) ? url : "";
    report.dataLink = (url, value, row) => report.safeLink(String(url || "").replace(/\$\{([^}]+)\}/g, (_, name) => encodeURIComponent(name === "value" ? value ?? "" : row[name] ?? "")));
    report.thresholdColor = (value, field) => {
        if (!finite(value)) return "";
        let color = "";
        let n = Number(value);
        if (field.thresholdMode === "percent") n = 100 * (n - (field.min ?? 0)) / ((field.max ?? 100) - (field.min ?? 0) || 1);
        for (const threshold of field.thresholds || []) if (threshold.value == null || n >= threshold.value) {
            color = /^(primary|secondary|accent|positive|negative|warning|info)$/.test(threshold.color) ? "var(--q-" + threshold.color + ")" : /^#[0-9a-f]{3,8}$/i.test(threshold.color || "") ? threshold.color : "";
        }
        return color;
    };
    report.sparkline = (values) => {
        const nums = (values || []).filter(finite).map(Number);
        if (!nums.length) return "";
        const min = Math.min(...nums), max = Math.max(...nums), length = values.length;
        let open = false;
        return values.map((value, at) => {
            if (!finite(value)) { open = false; return ""; }
            const point = (100 * at / Math.max(1, length - 1)).toFixed(2) + "," + (max === min ? 12 : 22 - 20 * (Number(value) - min) / (max - min)).toFixed(2);
            const command = open ? "L" : "M"; open = true; return command + point;
        }).join(" ");
    };
    report.mergeLogHits = (previous, incoming, limit) => {
        const byId = new Map();
        for (const hit of [...previous, ...incoming]) byId.set(hit.documentId + ":" + hit.id, hit);
        const hits = Array.from(byId.values());
        const removed = Math.max(0, hits.length - limit);
        return { hits: hits.slice(-limit), removed };
    };
    const components = moqui.qvtComponents = moqui.qvtComponents || {};
    report.parsePills = (value) => {
        try { const items=typeof value==='string'?JSON.parse(value || '[]'):value;return Array.isArray(items)?items.slice(0,32).filter(item=>item && typeof item==='object' && !Array.isArray(item)):[]; } catch (_) {return [];}
    };
    report.storage = {
        read(key,fallback) {try {return JSON.parse(root.localStorage.getItem(key)) ?? fallback;} catch (_) {return fallback;}},
        write(key,value) {try {root.localStorage.setItem(key,JSON.stringify(value));} catch (_) {/* Private browsing may disable storage. */}}
    };
    report.storageKey=(viewer,dataset)=>"moqui.report."+encodeURIComponent(JSON.stringify([String(viewer || "anonymous"),String(dataset || "dataset")]));
    report.suggestions = (query, fields, values) => {
        const token=String(query || '').match(/(?:^|\s)([^\s]*)$/),last=token?token[1]:'',colon=last.indexOf(':');
        if (colon>=0) {
            const name=last.slice(0,colon),prefix=last.slice(colon+1).replace(/^"/,'').toLowerCase();
            return (values && values[name] || []).filter(value=>String(value).toLowerCase().startsWith(prefix)).slice(0,12).map(value=>({text:name+':'+String(value),value:name+':'+JSON.stringify(value)}));
        }
        return [...(fields || []).filter(field=>field.searchable!==false).map(field=>({text:field.name+' ('+(field.type || 'string')+')',value:field.name+':'})),...['AND','OR','NOT'].map(value=>({text:value,value}))]
            .filter(item=>item.value.toLowerCase().startsWith(last.toLowerCase())).slice(0,12);
    };
    report.replaceQueryToken = (query,value) => String(query || '').replace(/(?:^|\s)[^\s]*$/,match=>(match.startsWith(' ')?' ':'')+value);
    components['m-report-querybar'] = {
        inject:{mForm:{default:null}},
        props:{fields:{type:Array,default:()=>[]},values:{type:Object,default:()=>({})},datasetId:String,viewerId:String},
        data:()=>({field:'',operation:'eq',value:'',label:'',editAt:-1,editJson:'',error:'',history:[],suggesting:false,choice:0}),
        computed:{
            formFields(){return this.mForm && this.mForm.fields || {};},
            query:{get(){return String(this.formFields.queryString || '');},set(value){this.formFields.queryString=value;}},
            pills(){return report.parsePills(this.formFields.filtersJson);},
            storageKey(){return report.storageKey(this.viewerId,this.datasetId);},
            suggestions(){return report.suggestions(this.query,this.fields,this.values);}
        },
        mounted(){this.restore();this.remember();},
        watch:{storageKey(){this.restore();}},
        methods:{
            restore(){this.history=report.storage.read(this.storageKey+'.history',[]);if(!Array.isArray(this.history))this.history=[];const pinned=report.parsePills(report.storage.read(this.storageKey+'.pins',[]));if(!this.pills.length && (!this.formFields.filtersJson || this.formFields.filtersJson==='[]') && pinned.length)this.write(pinned);},
            write(pills){this.formFields.filtersJson=JSON.stringify(pills.slice(0,32));report.storage.write(this.storageKey+'.pins',pills.filter(pill=>pill.pinned));},
            remember(){if(!this.query.trim())return;this.history=[this.query,...this.history.filter(query=>query!==this.query)].slice(0,20);report.storage.write(this.storageKey+'.history',this.history);},
            change(at,key){const pills=this.pills;if(pills[at]){pills[at][key]=!pills[at][key];this.write(pills);}},
            remove(at){const pills=this.pills;pills.splice(at,1);this.write(pills);},
            typed(value){const field=this.fields.find(field=>field.name===this.field);if(field && field.type==='number'){const number=Number(value);if(!String(value).trim() || !Number.isFinite(number))throw new Error('Enter a number');return number;}if(field && field.type==='boolean'){if(!['true','false'].includes(value))throw new Error('Enter true or false');return value==='true';}return value;},
            add(){try {if(!this.field)throw new Error('Choose a field');if(this.pills.length>=32)throw new Error('At most 32 filters');const field={name:this.field,op:this.operation};
                if(this.operation==='in')field.values=this.value.split(',').map(value=>this.typed(value.trim()));else if(this.operation==='range'){const values=this.value.split(',');if(values.length!==2)throw new Error('Enter lower,upper bounds');if(values[0].trim())field.gte=this.typed(values[0].trim());if(values[1].trim())field.lte=this.typed(values[1].trim());}else if(this.operation!=='exists')field.value=this.typed(this.value);
                this.write([...this.pills,{query:{field},label:this.label || '',disabled:false,negated:false,pinned:false}]);this.error='';this.value='';this.label='';
            }catch(error){this.error=String(error.message || error);}},
            edit(at){this.editAt=at;this.editJson=JSON.stringify(this.pills[at].query || {},null,2);this.label=this.pills[at].label || '';},
            saveEdit(){try {const query=JSON.parse(this.editJson);if(!query || Array.isArray(query) || typeof query!=='object' || Object.keys(query).length!==1)throw new Error('Enter one structured query object');const pills=this.pills;pills[this.editAt].query=query;pills[this.editAt].label=this.label;this.write(pills);this.editAt=-1;this.label='';this.error='';}catch(error){this.error=String(error.message || error);}},
            choose(item){this.query=report.replaceQueryToken(this.query,item.value);this.suggesting=false;this.choice=0;},
            key(event){if(event.key==='ArrowDown'){event.preventDefault();this.suggesting=true;this.choice=(this.choice+1)%Math.max(1,this.suggestions.length);}else if(event.key==='ArrowUp'){event.preventDefault();this.choice=(this.choice+this.suggestions.length-1)%Math.max(1,this.suggestions.length);}else if(event.key==='Escape'){this.suggesting=false;}else if(event.key==='Enter' && this.suggesting && this.suggestions[this.choice]){event.preventDefault();this.choose(this.suggestions[this.choice]);}else if(event.key==='Enter'){this.remember();}}
        },
        render(){const h=root.Vue.h;return h('section',{class:'q-pa-sm',onSubmit:this.remember},[
            h('label',['Query ',h('input',{type:'text',value:this.query,style:{width:'min(100%,60rem)'},'aria-label':'Search query','aria-autocomplete':'list','aria-expanded':this.suggesting,onInput:event=>{this.query=event.target.value;this.suggesting=true;this.choice=0;},onKeydown:this.key,onFocus:()=>{this.suggesting=true;},onBlur:()=>{root.setTimeout(()=>{this.suggesting=false;},150);this.remember();}})]),
            this.suggesting && this.suggestions.length?h('ul',{role:'listbox',class:'q-pa-xs'},this.suggestions.map((item,at)=>h('li',{role:'option','aria-selected':at===this.choice},[h('button',{type:'button',onClick:()=>this.choose(item)},item.text)]))):null,
            h('label',['Recent queries ',h('select',{'aria-label':'Query history',value:'',onChange:event=>{this.query=event.target.value;}},[h('option',{value:''},'Choose a query'),...this.history.map(query=>h('option',{value:query},query))])]),
            h('button',{type:'button',onClick:()=>{this.history=[];report.storage.write(this.storageKey+'.history',[]);}},'Clear query history'),
            h('div',{class:'row q-gutter-sm','aria-label':'Query filters'},this.pills.map((pill,at)=>h('div',{class:'q-pa-xs',style:{border:'1px solid var(--q-secondary)',opacity:pill.disabled?0.6:1}},[
                h('span',(pill.negated?'NOT ':'')+(pill.label || JSON.stringify(pill.query))+(pill.disabled?' (disabled)':'')),
                h('button',{type:'button','aria-pressed':!!pill.pinned,onClick:()=>this.change(at,'pinned')},pill.pinned?'Unpin':'Pin'),
                h('button',{type:'button','aria-pressed':!!pill.negated,onClick:()=>this.change(at,'negated')},'Negate'),
                h('button',{type:'button','aria-pressed':!!pill.disabled,onClick:()=>this.change(at,'disabled')},pill.disabled?'Enable':'Disable'),
                h('button',{type:'button',onClick:()=>this.edit(at)},'Edit query / label'),h('button',{type:'button',onClick:()=>this.remove(at)},'Remove')]))),
            this.editAt>=0?h('div',[h('label',['Filter label ',h('input',{value:this.label,onInput:event=>{this.label=event.target.value;}})]),h('label',['Structured query ',h('textarea',{value:this.editJson,rows:5,cols:70,onInput:event=>{this.editJson=event.target.value;}})]),h('button',{type:'button',onClick:this.saveEdit},'Apply filter edit'),h('button',{type:'button',onClick:()=>{this.editAt=-1;this.label='';}},'Cancel edit')]):null,
            h('details',[h('summary','Add filter'),h('label',['Field ',h('select',{value:this.field,onChange:event=>{this.field=event.target.value;}},[h('option',{value:''},'Choose field'),...this.fields.filter(field=>field.searchable!==false).map(field=>h('option',{value:field.name},field.name))])]),
                h('label',['Operation ',h('select',{value:this.operation,onChange:event=>{this.operation=event.target.value;}},['eq','in','exists','prefix','wildcard','range'].map(value=>h('option',{value},value)))]),
                h('label',['Value ',h('input',{value:this.value,disabled:this.operation==='exists',placeholder:this.operation==='range'?'lower,upper':this.operation==='in'?'value,value':'value',onInput:event=>{this.value=event.target.value;}})]),
                h('label',['Label ',h('input',{value:this.label,onInput:event=>{this.label=event.target.value;}})]),h('button',{type:'button',onClick:this.add},'Add filter')]),
            h('p','Pinned filters and query history are stored in this browser for this user and dataset. Search applies the filters.'),
            this.error?h('p',{role:'alert',class:'text-negative'},this.error):null]);}
    };

    components["m-report-definition-editor"] = {
        inject: { mForm: { default: null } },
        props: { name: { type: String, default: "definitionJson" }, libraries: { type: Array, default: () => [] }, datasets: { type: Array, default: () => [] }, readOnly: Boolean, panelOnly: Boolean },
        data: () => ({ selected: "", optionSearch: "", libraryId: "", error: "" }),
        computed: {
            body() { try { const value = JSON.parse(this.mForm && this.mForm.fields[this.name] || "{}"); return !value || Array.isArray(value) || typeof value !== "object" ? null : this.panelOnly ? { panels: { panel: value } } : value; } catch (_) { return null; } },
            panels() { return this.body && this.body.panels || {}; },
            selectedId() { return this.selected && this.panels[this.selected] ? this.selected : Object.keys(this.panels)[0]; }
        },
        methods: {
            write(body) { if (!this.readOnly && this.mForm) this.mForm.fields[this.name] = JSON.stringify(this.panelOnly ? body.panels.panel : body, null, 2); },
            update(path, value) {
                const body = this.body, id = this.selectedId;
                if (!body || !id || this.panels[id].panelRef) return;
                let target = body.panels[id];
                const parts = path.split(".");
                for (let at=0;at<parts.length-1;at++) { const key=parts[at];target=target[key] || (target[key]=/^\d+$/.test(parts[at+1])?[]:{}); }
                if (value === undefined) delete target[parts[parts.length - 1]];
                else target[parts[parts.length - 1]] = value;
                this.write(body);
            },
            add(copy) {
                const body = this.body;
                if (!body) return;
                body.panels = body.panels || {};
                let id = copy ? this.selectedId + "-copy" : "panel-" + Date.now().toString(36);
                while (body.panels[id]) id += "-copy";
                body.panels[id] = copy ? JSON.parse(JSON.stringify(body.panels[this.selectedId])) : { title: "New panel", visualization: "table", queries: [{ datasetId: "otel_metrics", mode: "documents", pageSize: 100 }] };
                this.selected = id; this.write(body);
            },
            remove() { const body = this.body; if (body && this.selectedId) { delete body.panels[this.selectedId]; this.write(body); } },
            library(unlink) {
                const body = this.body;
                if (!body) return;
                const reference = unlink ? this.panels[this.selectedId].panelRef : this.libraryId;
                const definition = this.libraries.find((item) => item.id === reference);
                if (!definition) return;
                body.panels = body.panels || {};
                const id = unlink ? this.selectedId : "library-" + Date.now().toString(36);
                body.panels[id] = unlink ? JSON.parse(JSON.stringify(definition.body)) : { panelRef: definition.id };
                this.selected = id; this.write(body);
            },
            layout(type) {
                const body = this.body;
                if (!body) return;
                const ids = Object.keys(body.panels || {});
                const auto = { type: "autoGrid", items: ids.map((panelId) => ({ panelId })) };
                body.layout = type === "grid" ? { type, items: ids.map((panelId, at) => ({ panelId, x: at % 2 * 12, y: Math.floor(at / 2), w: 12, h: 1 })) } :
                    type === "rows" ? { type, items: [{ title: "Row", layout: auto }] } : type === "tabs" ? { type, items: [{ title: "Tab", layout: auto }] } : auto;
                this.write(body);
            },
            aggregation(add, at) {
                const body=this.body,panel=body && body.panels[this.selectedId];if (!panel || panel.panelRef) return;
                panel.queries=panel.queries || [{}];const query=panel.queries[0],items=query.aggregations=query.aggregations || [];
                if (add) { query.mode='aggregations';items.push({id:'metric'+(items.length+1),type:'sum',role:'metric',params:{field:'sum'}}); }
                else items.splice(at,1);this.write(body);
            },
            transformation(add, at) {
                const body=this.body,panel=body && body.panels[this.selectedId];if (!panel || panel.panelRef) return;
                const items=panel.transformations=panel.transformations || [];
                if (add) items.push({type:'limit',count:100});else items.splice(at,1);this.write(body);
            },
            position(key, value) {
                const body = this.body;
                if (!body || !body.layout || body.layout.type !== "grid") return;
                const item = (body.layout.items || []).find((item) => item.panelId === this.selectedId);
                if (item) { item[key] = Number(value); this.write(body); }
            }
        },
        render() {
            const h = root.Vue.h;
            if (!this.body) return h("p", { role: "alert" }, "Correct the JSON below to use the panel options.");
            const panel = this.panels[this.selectedId] || {}, options = [];
            const read = (path) => path.split(".").reduce((value, key) => value && value[key], panel);
            const input = (label, path, type, choices) => {
                if (this.optionSearch && !label.toLowerCase().includes(this.optionSearch.toLowerCase())) return;
                options.push(h("label", { class: "q-pa-xs", style: { display: "block" } }, [h("span", { style: { display: "block" } }, label), choices ?
                    h("select", { value: read(path) || "", disabled: this.readOnly, onChange: (event) => this.update(path, event.target.value) }, choices.map((choice) => h("option", { value: choice.value ?? choice }, choice.text || choice.value || choice))) :
                    h("input", { type: type || "text", checked: type === "checkbox" ? !!read(path) : undefined, value: read(path) ?? "", disabled: this.readOnly, onChange: (event) => this.update(path, type === "checkbox" ? event.target.checked : type === "number" ? (event.target.value === "" ? undefined : Number(event.target.value)) : event.target.value) })]));
            };
            input("Panel title", "title");
            input("Visualization", "visualization", "select", ["timeSeries", "area", "bar", "stat", "gauge", "barGauge", "table", "pie", "donut", "heatmap", "stateTimeline", "logs", "traceWaterfall", "text"]);
            input("Dataset", "queries.0.datasetId", "select", this.datasets.map((dataset) => ({ value: dataset.id, text: dataset.title || dataset.id })));
            input("Query string", "queries.0.queryString");
            input("Query mode", "queries.0.mode", "select", ["aggregations", "documents"]);
            const aggregationRows=((panel.queries || [])[0] || {}).aggregations || [];
            aggregationRows.forEach((aggregation,at)=>{
                const prefix='queries.0.aggregations.'+at+'.';
                input('Aggregation '+(at+1)+' ID',prefix+'id');
                input('Aggregation '+(at+1)+' type',prefix+'type','select',['value_count','sum','min','max','avg','terms','date_histogram','cardinality','percentiles','histogram_merge']);
                input('Aggregation '+(at+1)+' role',prefix+'role','select',['metric','x-axis','series']);
                input('Aggregation '+(at+1)+' field',prefix+'params.field');
                if (aggregation.type==='terms') input('Aggregation '+(at+1)+' top values',prefix+'params.size','number');
                if (aggregation.type==='date_histogram') input('Aggregation '+(at+1)+' interval',prefix+'params.interval');
                options.push(h('button',{type:'button',disabled:this.readOnly,onClick:()=>this.aggregation(false,at)},'Remove aggregation '+(at+1)));
            });
            options.push(h('button',{type:'button',disabled:this.readOnly,onClick:()=>this.aggregation(true)},'Add aggregation'));
            (panel.transformations || []).forEach((step,at)=>{
                const prefix='transformations.'+at+'.';
                input('Transformation '+(at+1)+' type',prefix+'type','select',['limit','sort','reduce','filter','calculate']);
                input('Transformation '+(at+1)+' disabled',prefix+'disabled','checkbox');
                if (['filter','sort'].includes(step.type)) input('Transformation '+(at+1)+' field',prefix+'field');
                if (step.type==='sort') input('Transformation '+(at+1)+' descending',prefix+'descending','checkbox');
                if (step.type==='limit') input('Transformation '+(at+1)+' row limit',prefix+'count','number');
                if (step.type==='reduce') input('Transformation '+(at+1)+' calculation',prefix+'calculation','select',['sum','avg','min','max','count','first','last']);
                if (step.type==='filter') { input('Transformation '+(at+1)+' operator',prefix+'operator','select',['eq','ne','gt','lt','exists']);input('Transformation '+(at+1)+' value',prefix+'value'); }
                if (step.type==='calculate') { input('Transformation '+(at+1)+' output field',prefix+'name');input('Transformation '+(at+1)+' left field',prefix+'left');input('Transformation '+(at+1)+' right field',prefix+'right');input('Transformation '+(at+1)+' constant',prefix+'value','number');input('Transformation '+(at+1)+' operator',prefix+'operator','select',['+','-','*','/']); }
                options.push(h('button',{type:'button',disabled:this.readOnly,onClick:()=>this.transformation(false,at)},'Remove transformation '+(at+1)));
            });
            options.push(h('button',{type:'button',disabled:this.readOnly,onClick:()=>this.transformation(true)},'Add transformation'));
            input("Field unit", "fieldConfig.defaults.unit");
            input("Decimal places", "fieldConfig.defaults.decimals", "number");
            input("Field minimum", "fieldConfig.defaults.min", "number");
            input("Field maximum", "fieldConfig.defaults.max", "number");
            input("No-value text", "fieldConfig.defaults.noValue");
            input("Compare previous period", "comparePrevious", "checkbox");
            input("Panel time from override", "time.from"); input("Panel time through override", "time.thru");
            const layout = this.body.layout || {};
            if (layout.type === "grid") {
                const item = (layout.items || []).find((item) => item.panelId === this.selectedId) || {};
                for (const [key, label] of [["x", "Grid column (0–23)"], ["y", "Grid row"], ["w", "Width in columns (1–24)"], ["h", "Height in rows"]]) {
                    if (!this.optionSearch || label.toLowerCase().includes(this.optionSearch.toLowerCase())) options.push(h("label", { class: "q-pa-xs" }, [label,
                        h("input", { type: "number", value: item[key] ?? "", disabled: this.readOnly, onChange: (event) => this.position(key, event.target.value) })]));
                }
            }
            return h("section", { class: "q-pa-sm", style: { border: "1px solid var(--q-secondary)" } }, [h("h4", "Panel options"),
                this.panelOnly ? null : h("div", { class: "row q-gutter-sm" }, [h("select", { value: this.selectedId || "", onChange: (event) => { this.selected = event.target.value; } }, Object.keys(this.panels).map((id) => h("option", { value: id }, this.panels[id].title || id))),
                    h("button", { type: "button", disabled: this.readOnly, onClick: () => this.add(false) }, "Add panel"),
                    h("button", { type: "button", disabled: this.readOnly || !this.selectedId, onClick: () => this.add(true) }, "Duplicate"),
                    h("button", { type: "button", disabled: this.readOnly || !this.selectedId, onClick: this.remove }, "Remove"),
                    h("label", ["Layout ", h("select", { value: layout.type || "autoGrid", disabled: this.readOnly, onChange: (event) => this.layout(event.target.value) }, ["autoGrid", "grid", "rows", "tabs"].map((value) => h("option", { value }, value)))]) ]),
                this.panelOnly ? null : h("div", { class: "q-py-sm" }, [h("label", ["Library panel ", h("select", { value: this.libraryId, onChange: (event) => { this.libraryId = event.target.value; } }, [h("option", { value: "" }, "Choose"), ...this.libraries.map((item) => h("option", { value: item.id }, item.title))])]),
                    h("button", { type: "button", disabled: this.readOnly || !this.libraryId, onClick: () => this.library(false) }, "Add library reference")]),
                panel.panelRef ? h("p", ["Library panel: " + panel.panelRef + " ", h("a", { href: "/apps/dashboards/Definition?definitionId=" + encodeURIComponent(panel.panelRef) }, "Edit library panel"), " ",
                    h("button", { type: "button", disabled: this.readOnly, onClick: () => this.library(true) }, "Unlink as copy")]) :
                    h("div", [h("input", { type: "search", placeholder: "Search panel options", value: this.optionSearch, onInput: (event) => { this.optionSearch = event.target.value; } }), ...options]),
                h("p", "Options update the definition below. Save a version to keep the changes. Queries, transformations and nested layouts can also be edited in JSON.")]);
        }
    };
    components["m-log-tail"] = {
        props: { url: { type: String, required: true }, params: { type: Object, default: () => ({}) } },
        data: () => ({ active: false, hits: [], cursor: null, error: "", removed: 0, timer: null, loading: false }),
        beforeUnmount() { root.clearInterval(this.timer); },
        methods: {
            toggle() {
                this.active = !this.active; root.clearInterval(this.timer);
                if (this.active) { this.poll(); this.timer = root.setInterval(() => { if (!root.document.hidden) this.poll(); }, 2000); }
            },
            async poll() {
                if (this.loading) return;
                this.loading = true;
                try {
                    const result = await moqui.request(this.url, { params: Object.assign({}, this.params, this.cursor ? { cursor: JSON.stringify(this.cursor) } : {}) });
                    if (!result.json || result.status >= 400) throw new Error(result.json && result.json.error || "Telemetry request failed");
                    const merged = report.mergeLogHits(this.hits, result.json.hits || [], 500);
                    this.hits = merged.hits; this.removed += merged.removed;
                    this.cursor = result.json.nextCursor || this.cursor; this.error = "";
                } catch (error) { this.error = String(error); }
                finally { this.loading = false; }
            }
        },
        render() {
            const h = root.Vue.h;
            return h("section", { class: "q-pa-sm" }, [h("button", { type: "button", class: "q-btn q-pa-xs", "aria-pressed": this.active, onClick: this.toggle }, this.active ? "Pause live tail" : "Start live tail"),
                this.error ? h("p", { role: "alert", class: "text-negative" }, this.error + " — telemetry is unavailable") : null,
                this.removed ? h("p", this.removed + " older entries removed from this view; search or export to read them") : null,
                this.active || this.hits.length ? h("div", { role: "log", "aria-live": "off", style: { maxHeight: "420px", overflow: "auto", fontFamily: "monospace" } },
                    this.hits.length ? this.hits.map((hit) => {
                        const doc = hit.body || {}, trace = doc.trace_id;
                        return h("details", { key: hit.documentId + ":" + hit.id, class: "q-py-xs" }, [
                            h("summary", { style: { color: Number(doc.severity_number) >= 17 ? "var(--q-negative)" : undefined } },
                                report.format(doc.timestamp || doc.start, { type: "time" }) + " " + (doc.severity_text || "") + " " + String(doc.body || doc.name || "").slice(0, 500)),
                            trace ? h("a", { href: "/apps/monitor/TraceDetail?traceId=" + encodeURIComponent(trace) }, "Open trace") : null,
                            h("pre", { style: { whiteSpace: "pre-wrap", overflowWrap: "anywhere" } }, JSON.stringify(doc, null, 2)) ]);
                    }) : [h("p", "No new telemetry. This does not establish health.")]) : null ]);
        }
    };
    components["m-trace-waterfall"] = {
        props: { spans: { type: Array, default: () => [] } }, emits: ["select"],
        data: () => ({ selected: null }),
        render() {
            const h = root.Vue.h, rows = report.waterfall(this.spans);
            if (!rows.length) return h("p", { role: "status" }, "No spans in this time range. Telemetry may be missing.");
            return h("div", { class: "m-trace-waterfall" }, [
                ...rows.map((row) => h("button", { type: "button", class: "full-width text-left q-pa-sm", key: row.span.span_id,
                    style: { display: "grid", gridTemplateColumns: "minmax(180px, 35%) 1fr", border: "0", background: "transparent", color: "inherit" },
                    onClick: () => { this.selected = row.span; this.$emit("select", row.span); },
                    "aria-label": row.span.name + ", " + row.duration.toFixed(2) + " milliseconds" }, [
                    h("span", { style: { paddingLeft: row.depth * 12 + "px", overflow: "hidden", textOverflow: "ellipsis" } },
                        row.span.name + (row.missingParent ? " (parent unavailable)" : "")),
                    h("span", { style: { position: "relative", height: "22px", borderBottom: "1px solid var(--q-secondary)" } }, [
                        h("span", { style: { position: "absolute", left: row.left + "%", width: row.width + "%", minWidth: "2px", height: "18px",
                            background: row.span.status_code === "ERROR" ? "var(--q-negative)" : "var(--q-primary)", borderRadius: "2px" } }),
                        h("span", { style: { position: "relative", textShadow: "0 0 3px var(--q-dark)", paddingLeft: "4px" } }, row.duration.toFixed(2) + " ms") ]) ])),
                this.selected ? h("details", { open: true, class: "q-pa-sm" }, [h("summary", this.selected.name), h("pre", { style: { whiteSpace: "pre-wrap", overflowWrap: "anywhere" } }, JSON.stringify(this.selected, null, 2))]) : null ]);
        }
    };
    components["m-report-playlist"] = {
        props: { items: {type:Array,default:()=>[]}, seconds: {type:Number,default:30} },
        data: () => ({ at:0, paused:false, timer:null }),
        mounted() { this.timer=root.setInterval(()=>{ if (!this.paused && !root.document.hidden && this.items.length) this.at=(this.at+1)%this.items.length; }, Math.max(5,this.seconds)*1000); },
        beforeUnmount() { if (this.timer) root.clearInterval(this.timer); },
        render() {
            const h=root.Vue.h,item=this.items[this.at];
            if (!item) return h("p",{role:"status"},"No accessible dashboards in this playlist.");
            return h("section",[h("div",{class:"row items-center q-gutter-sm"},[
                h("button",{type:"button",onClick:()=>{this.at=(this.at+this.items.length-1)%this.items.length;}},"Previous"),
                h("button",{type:"button",onClick:()=>{this.paused=!this.paused;}},this.paused?"Resume":"Pause"),
                h("button",{type:"button",onClick:()=>{this.at=(this.at+1)%this.items.length;}},"Next"),
                h("a",{href:report.safeLink(item.url)},item.title),h("span",(this.at+1)+" / "+this.items.length)]),
                h("iframe",{key:this.at,src:report.safeLink(item.url),title:item.title,style:{width:"100%",height:"80vh",border:0},referrerpolicy:"same-origin"})]);
        }
    };
    components["m-report-panel"] = {
        props: { frame: { type: Object, default: () => ({ fields: [] }) }, frames: Array, panel: { type: Object, default: () => ({}) },
            title: String, height: { type: String, default: "300px" }, dashboardId: String, panelId: String, range: Object, variables: Object, syncGroup:String },
        data: () => ({ sortField: -1, descending: false, inspector: false, inspection: null, busy: false, actionError: "" }),
        methods: {
            async panelRequest(action, extra) {
                this.busy = true; this.actionError = "";
                try {
                    const parameters = new root.URLSearchParams(Object.assign({ definitionId: this.dashboardId, panelId: this.panelId,
                        from: this.range && this.range.from || "now-7d", thru: this.range && this.range.thru || "now", variables: JSON.stringify(this.variables || {}) }, extra || {}));
                    const response = await root.fetch("/apps/dashboards/Dashboard/" + action + "?" + parameters.toString(), { credentials: "same-origin", headers: { Accept: "application/json" } });
                    if (!response.ok) throw new Error("Request failed (" + response.status + ")");
                    const result = await response.json();
                    if (result.errors || result.error) throw new Error(result.errors || result.error);
                    return result;
                } catch (error) { this.actionError = String(error.message || error); return null; }
                finally { this.busy = false; }
            },
            async inspect() {
                this.inspector = !this.inspector;
                if (this.inspector && this.dashboardId && !this.inspection) this.inspection = await this.panelRequest("inspectPanel");
            },
            async exportPanel(renderMode) {
                const result = await this.panelRequest("exportPanel", { renderMode });
                if (!result) return;
                if (!["text/csv", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"].some(type => String(result.contentType).startsWith(type))) { this.actionError = "Unsupported export response"; return; }
                const bytes = root.atob(result.contentBase64 || ""), data = new Uint8Array(bytes.length);
                for (let at = 0; at < bytes.length; at++) data[at] = bytes.charCodeAt(at);
                const url = root.URL.createObjectURL(new root.Blob([data], { type: result.contentType })), link = root.document.createElement("a");
                link.href = url; link.download = result.filename || "panel." + renderMode; link.click(); root.URL.revokeObjectURL(url);
            },
            csv() { const blob = new root.Blob([report.csv(this.frames && this.frames[0] || this.frame)], { type: "text/csv;charset=utf-8" });
                const url = root.URL.createObjectURL(blob), link = root.document.createElement("a"); link.href = url; link.download = (this.title || this.panel.title || "report") + ".csv";
                link.click(); root.URL.revokeObjectURL(url); }
        },
        render() {
            const h = root.Vue.h, frame = this.frames && this.frames[0] || this.frame;
            const fields = report.configuredFields(frame, this.panel), rows = report.rows(frame), visual = this.panel.visualization || this.panel.type || "table";
            const title = this.title || this.panel.title;
            const header = h("div", { class: "row items-center q-gutter-sm" }, [h("strong", { class: "col" }, title || "Report"),
                h("button", { type: "button", onClick: this.dashboardId ? () => this.exportPanel("csv") : this.csv, disabled: this.busy, class: "q-btn q-pa-xs" }, "CSV"),
                this.dashboardId ? h("button", { type: "button", onClick: () => this.exportPanel("xlsx"), disabled: this.busy, class: "q-btn q-pa-xs" }, "XLSX") : null,
                h("button", { type: "button", onClick: this.inspect, disabled: this.busy, class: "q-btn q-pa-xs" }, "Inspect"),
                this.dashboardId && (this.panel.queries || []).some((query) => (query.aggregations || []).some((agg) => agg.role === "metric")) ?
                    h("a", { href: "/apps/dashboards/AlertRule?dashboardId=" + encodeURIComponent(this.dashboardId) + "&panelId=" + encodeURIComponent(this.panelId) }, "New alert rule") : null]);
            let content;
            if (visual === "text") content = h(root.Vue.resolveComponent("m-markdown"), { text: this.panel.text || "" });
            else if (!rows.length) content = h("p", { role: "status", class: "text-grey" }, "No data in this time range.");
            else if (visual === "stat" || visual === "gauge" || visual === "barGauge") {
                const at = fields.findIndex((field) => !report.isMetadata(field) && (field.type || field.kind) === "number"), field = fields[at] || fields[0], value = rows[rows.length - 1][at < 0 ? 0 : at];
                const previous = this.panel.compareField ? rows[rows.length - 1][fields.findIndex((item) => item.name === this.panel.compareField)] : null;
                const spark = report.sparkline(field.values || []);
                content = h("div", { class: "q-pa-md" }, [h("div", { class: "text-h4", style: { color: report.thresholdColor(value, field) } }, report.format(value, field)),
                    previous && finite(value) ? h("p", ((Number(value) - Number(previous)) / Math.abs(Number(previous)) * 100).toFixed(1) + "% from previous period") : null,
                    visual !== "stat" ? h("meter", { min: field.min ?? 0, max: field.max ?? 100, value, style: { width: "100%" } }) :
                        h("svg", { viewBox: "0 0 100 24", height: "36", width: "100%", role: "img", "aria-label": "Recent values" }, [h("path", { d: spark, fill: "none", stroke: "var(--q-primary)", "stroke-width": "1" })])]);
            } else if (["timeSeries", "timeseries", "line", "area", "bar", "pie", "donut"].includes(visual)) {
                content = h(root.Vue.resolveComponent("m-chart"), { config: report.chart(frame, Object.assign({},this.panel,{syncGroup:this.syncGroup})), height: this.height });
            } else if (visual === "traceWaterfall" || visual === "trace") {
                content = h(components["m-trace-waterfall"], { spans: rows.map((row) => Object.fromEntries(fields.map((field, at) => [field.name, row[at]]))) });
            } else if (visual === "heatmap") {
                const values = rows.map((row) => row.filter(finite).map(Number));
                const max = Math.max(1, ...values.flat());
                content = h("div", { style: { display: "grid", gridTemplateColumns: "repeat(" + Math.max(1, fields.length) + ",1fr)", gap: "2px" } },
                    rows.flatMap((row, at) => row.map((value, index) => h("div", { key: at + ":" + index, title: fields[index].name + ": " + report.format(value, fields[index]),
                        style: { padding: "4px", background: finite(value) ? "var(--q-primary)" : "transparent", opacity: finite(value) ? Math.max(0.15, Number(value) / max) : 1 } }, report.format(value, fields[index])))));
            } else if (visual === "stateTimeline") {
                content = h("div", rows.map((row, at) => h("div", { key: at, class: "row no-wrap" }, row.map((value, index) => h("span", {
                    style: { padding: "8px", flex: index ? 1 : undefined, background: index ? report.thresholdColor(value, fields[index]) || "var(--q-secondary)" : "transparent" },
                    title: fields[index].name + ": " + String(value ?? "Missing") }, report.format(value, fields[index]))))));
            } else {
                const visible = fields.map((field, index) => ({ field, index })).filter(({ field }) => !field.hidden && !(field.config && field.config.hidden));
                const sorted = rows.map((row) => row.slice());
                if (this.sortField >= 0) sorted.sort((a, b) => {
                    const av = a[this.sortField], bv = b[this.sortField];
                    const n = av == null ? (bv == null ? 0 : 1) : bv == null ? -1 : finite(av) && finite(bv) ? Number(av) - Number(bv) : String(av).localeCompare(String(bv));
                    return this.descending ? -n : n;
                });
                content = h("div", { class: "scroll" }, [h("table", { class: "q-table full-width" }, [
                    h("thead", [h("tr", visible.map(({ field, index }) => h("th", { style: { textAlign: "left" }, "aria-sort": this.sortField === index ? (this.descending ? "descending" : "ascending") : "none" }, [
                        h("button", { type: "button", onClick: () => { this.descending = this.sortField === index ? !this.descending : false; this.sortField = index; } }, field.displayName || field.name) ])))]),
                    h("tbody", sorted.map((row, at) => h("tr", { key: at }, visible.map(({ field, index }) => {
                        const value = row[index], text = report.format(value, field), record = Object.fromEntries(fields.map((item, i) => [item.name, row[i]]));
                        const link = field.link ? report.dataLink(field.link, value, record) : "";
                        return h("td", { style: { color: report.thresholdColor(value, field), ...(field.frozen ? { position: "sticky", left: "0", background: "var(--q-dark)" } : {}) } },
                            link ? h("a", { href: link }, text) : text.length > 240 ? h("details", [h("summary", text.slice(0, 200) + "…"), h("pre", { style: { whiteSpace: "pre-wrap" } }, text)]) : text);
                    })))),
                    this.panel.totals ? h("tfoot", [h("tr", visible.map(({ field, index }) => h("td", report.format(this.panel.totals[field.name] ?? null, field))))]) : null ]) ]);
            }
            const annotations = (this.panel.annotations || []).slice(0, 200);
            return h("section", { class: "m-report-panel q-pa-sm q-mb-sm", style: { border: "1px solid var(--q-secondary)", borderRadius: "4px" } },
                [header, this.actionError ? h("p", { role: "alert", class: "text-negative" }, this.actionError) : null, content, annotations.length ? h("details", [h("summary", annotations.length + " annotations"), h("ul", annotations.map((annotation) => h("li", report.format(annotation.time, { type: "time" }) + " · " + annotation.text)))]) : null,
                this.inspector ? h("pre", { style: { whiteSpace: "pre-wrap", overflowWrap: "anywhere" } }, JSON.stringify(this.inspection || { panel: this.panel, frame }, null, 2)) : null]);
        }
    };
    components["m-dashboard"] = {
        props: { dashboard: { type: Object, default: () => ({ panels: {} }) }, frames: { type: Object, default: () => ({}) } },
        data: () => ({ syncGroup:"report-dashboard-"+Math.random().toString(36).slice(2), keyPrefix:"", keyAt:0, panelRefs:{}, selectedPanel:"", activeTabs: {}, kiosk: false, refreshSeconds: 0, timer: null }),
        mounted() {
            this.refreshSeconds = Number(this.dashboard.refreshSeconds) || 0;
            this.setRefresh();
            this.onKey = (event) => {
                if (/INPUT|TEXTAREA|SELECT/.test(event.target.tagName) || event.target.isContentEditable) return;
                if (event.key === "k" && event.altKey) { event.preventDefault(); this.kiosk = !this.kiosk;return; }
                const prefix=Date.now()-this.keyAt<2000?this.keyPrefix:"";this.keyPrefix="";
                if (prefix==='t') {const range=report.shiftRange(this.dashboard.absoluteTime,event.key);if(range){event.preventDefault();this.timeRange(range);return;}}
                if (prefix==='d' && event.key==='k') {event.preventDefault();this.kiosk=!this.kiosk;return;}
                if (event.ctrlKey || event.metaKey || event.altKey)return;
                if (event.key==='t'||event.key==='d') {this.keyPrefix=event.key;this.keyAt=Date.now();event.preventDefault();return;}
                const panels=this.dashboard.panels || {},id=this.selectedPanel || Object.keys(panels)[0];
                if (event.key==='i' && this.panelRefs[id]) {event.preventDefault();this.panelRefs[id].inspect();}
                if (event.key==='v' && id && this.dashboard.definitionId) {event.preventDefault();const url=new root.URL(root.location.href);url.searchParams.set('panelId',id);root.location.assign(url.href);}
                if (event.key==='e' && this.dashboard.definitionId) {event.preventDefault();root.location.assign('/apps/dashboards/Definition?definitionId='+encodeURIComponent(this.dashboard.definitionId));}
            };
            root.document.addEventListener("keydown", this.onKey);
            this.onRange=(event)=>{if(event.detail.group!==this.syncGroup)return;const url=new root.URL(root.location.href);url.searchParams.set('from',event.detail.from);url.searchParams.set('thru',event.detail.thru);url.searchParams.delete('cursor');url.searchParams.delete('pageIndex');root.location.assign(url.href);};
            root.document.addEventListener('moqui-report-range',this.onRange);
        },
        beforeUnmount() { root.clearInterval(this.timer); root.document.removeEventListener("keydown", this.onKey);root.document.removeEventListener("moqui-report-range",this.onRange); },
        methods: {
            timeRange(range) {const url=new root.URL(root.location.href);url.searchParams.set('from',range.from);url.searchParams.set('thru',range.thru);url.searchParams.delete('cursor');url.searchParams.delete('pageIndex');root.location.assign(url.href);},
            setRefresh() {
                root.clearInterval(this.timer);
                if (this.refreshSeconds >= 5) this.timer = root.setInterval(() => {
                    if (!root.document.hidden) root.location.reload();
                }, this.refreshSeconds * 1000);
            },
            variable(variable, value) {
                const url = new root.URL(root.location.href), name = "var-" + variable.name;
                url.searchParams.delete(name);
                for (const item of Array.isArray(value) ? value : [value]) url.searchParams.append(name, item);
                root.location.assign(url.href);
            },
            share() {
                const url = new root.URL(root.location.href), time = this.dashboard.absoluteTime || {};
                if (time.from) url.searchParams.set("from", time.from);
                if (time.thru) url.searchParams.set("thru", time.thru);
                if (root.navigator.clipboard) root.navigator.clipboard.writeText(url.href);
            }
        },
        render() {
            const h = root.Vue.h, panels = this.dashboard.panels || {};
            const entries = Array.isArray(panels) ? panels.map((panel, at) => [panel.id || String(at), panel]) : Object.entries(panels);
            const byId = Object.fromEntries(entries);
            const panel = (id, position) => byId[id] ? h("div", { key: id, style: position || {},onFocusin:()=>{this.selectedPanel=id;},onMouseenter:()=>{this.selectedPanel=id;} }, [h(components["m-report-panel"], { ref:instance=>{this.panelRefs[id]=instance;},panel: byId[id], frames: this.frames[id] || byId[id].frames,
                frame: byId[id].frame || { fields: [] }, dashboardId: this.dashboard.definitionId, panelId: id, range: this.dashboard.absoluteTime, variables: this.dashboard.variableValues, syncGroup:this.syncGroup })]) : h("p", { role: "alert" }, "Panel " + id + " is unavailable");
            const auto = (items) => h("div", { style: { display: "grid", gridTemplateColumns: "repeat(auto-fit,minmax(min(100%,360px),1fr))", gap: "12px" } }, items);
            const layout = (node, path) => {
                if (!node) return auto(entries.map(([id]) => panel(id)));
                if (node.panelId) return panel(node.panelId);
                const children = node.items || node.children || [];
                if (node.type === "grid") return h("div", { style: { display: "grid", gridTemplateColumns: "repeat(24,minmax(0,1fr))", gap: "12px" } }, children.map((item) => panel(item.panelId,
                    { gridColumn: (Math.max(0, Math.min(23, item.x || 0)) + 1) + "/span " + Math.max(1, Math.min(24, item.w || 12)), gridRow: (Math.max(0, item.y || 0) + 1) + "/span " + Math.max(1, item.h || 1), minWidth: "0" })));
                if (node.type === "tabs") {
                    const at = this.activeTabs[path] || 0;
                    return h("section", [h("div", { role: "tablist" }, children.map((item, index) => h("button", { type: "button", role: "tab", "aria-selected": at === index,
                        onClick: () => { this.activeTabs[path] = index; }, class: "q-btn q-pa-sm" }, item.title || "Tab " + (index + 1)))),
                    children[at] ? h("div", { role: "tabpanel" }, [layout(children[at].layout || children[at], path + ":" + at)]) : null]);
                }
                if (node.type === "rows" || node.type === "row") return h("div", children.map((item, index) => h("details", { open: item.collapsed !== true }, [
                    h("summary", { class: "q-pa-sm text-weight-bold" }, item.title || "Row " + (index + 1)), layout(item.layout || { type: "autoGrid", items: item.items || [] }, path + ":" + index) ])));
                return auto(children.length ? children.map((item, index) => typeof item === "string" ? panel(item) : layout(item, path + ":" + index)) : entries.map(([id]) => panel(id)));
            };
            const variables = (this.dashboard.variables || []).filter((variable) => variable.kind !== "constant").map((variable) => {
                const value = this.dashboard.variableValues && this.dashboard.variableValues[variable.name] || variable.default || "";
                const options = variable.options || [];
                return h("label", { class: "q-pa-xs" }, [h("span", variable.label || variable.name), options.length ? h("select", { multiple: !!variable.multi,
                    value, onChange: (event) => this.variable(variable, variable.multi ? Array.from(event.target.selectedOptions).map((option) => option.value) : event.target.value) },
                    [...(variable.includeAll ? [h("option", { value: "$__all" }, "All")] : []), ...options.map((item) => h("option", { value: typeof item === "object" ? item.value : item }, typeof item === "object" ? item.text || item.value : item))]) :
                    h("input", { value: Array.isArray(value) ? value.join(",") : value, onChange: (event) => this.variable(variable, event.target.value) })]);
            });
            const toolbar = this.kiosk ? null : h("div", { class: "row items-center q-gutter-sm q-py-sm" }, [...variables,
                h("label", ["Refresh ", h("select", { value: this.refreshSeconds, onChange: (event) => { this.refreshSeconds = Number(event.target.value); this.setRefresh(); } },
                    [0, 5, 15, 30, 60, 300].map((seconds) => h("option", { value: seconds }, seconds ? seconds + "s" : "Off")))]),
                h("button", { type: "button", class: "q-btn q-pa-xs", onClick: () => root.location.reload() }, "Refresh now"),
                h("span", "Charts share a cursor. Drag a time range to zoom all panels; double click to reset chart zoom."),
                h("button", { type: "button", class: "q-btn q-pa-xs", onClick: this.share }, "Copy absolute-time link"),
                h("button", { type: "button", class: "q-btn q-pa-xs", onClick: () => { this.kiosk = true; } }, "Kiosk (Alt+K)") ]);
            return h("div", { class: "m-report-dashboard", style: this.kiosk ? { position: "fixed", inset: "0", zIndex: "10000", overflow: "auto", background: "var(--q-dark,white)", padding: "12px" } : {} },
                [toolbar, this.kiosk ? h("button", { type: "button", onClick: () => { this.kiosk = false; } }, "Exit kiosk") : null, layout(this.dashboard.layout, "root")]);
        }
    };
    if (moqui.qvt && moqui.qvt.registerLate) Object.entries(components).filter(([name]) => ["m-report-querybar", "m-report-playlist", "m-report-panel", "m-dashboard", "m-trace-waterfall", "m-log-tail", "m-report-definition-editor"].includes(name))
        .forEach(([name, component]) => moqui.qvt.registerLate(name, component));
})(typeof window !== "undefined" ? window : globalThis);
