/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! The facade calls the tools screens and services make that are not part of the Server host methods: cache admin,
//! the entity and service model, screen statistics, the server status, the SQL runner, and the data writer and
//! loader builders. The Rust functions return plain data; [`TOOLS_JS`] puts the Java-shaped methods on it
//! (`getEntityDefinition(name).getPrettyName(...)`, `workerPool.getActiveCount()`, `makeDataWriter().file(...)`).
//!
//! A method does not survive the context stack, which holds data, so each object also has its getters as data
//! (`prettyName`, `activeCount`), which is what a screen reads once it has `set` the object in a field.

use super::*;
use moqui_lib::node::MNode;

const TOOLS_JS: &str = r#"(function (ec, cu, su, wu, host) {
    // ---- collections ----
    // `context` is a read-only view of the stack here, so the page values are set on the stack by name.
    cu.paginateList = function (listName, pageListName, context) {
        pageListName = pageListName || listName;
        if (context[pageListName + "AlreadyPaginated"] != null) return;
        const theList = Array.from(context[listName] || []);
        const values = Object.assign({}, context);
        if (values[pageListName + "PageRangeLow"] == null || values[pageListName + "PageRangeHigh"] == null) {
            cu.paginateParameters(theList.length, pageListName, values);
            for (const suffix of ["Count", "PageIndex", "PageSize", "PageMaxIndex", "PageRangeLow", "PageRangeHigh"])
                globalThis[pageListName + suffix] = values[pageListName + suffix];
        }
        const low = values[pageListName + "PageRangeLow"], high = values[pageListName + "PageRangeHigh"];
        globalThis[pageListName] = theList.slice(Math.max(low - 1, 0), high);
    };
    cu.orderMapList = function (list, fields) {
        orderMapList(list, fields);
        return list;
    };

    // ---- wrappers: a tools object is data with a `__wrap` name; reading it from the stack adds its methods back ----
    const wrappers = {};
    const wrap = (name, data) => { if (data == null) return null; data.__wrap = name; return wrappers[name](data); };
    globalThis.__moquiWrap = (name, data) => (wrappers[name] ? wrappers[name](data) : data);

    // the parts of MNode the screens call: `{name, attributes, text, children, xml}`
    wrappers.Node = function (node) {
        node.attributes = node.attributes || {};
        // the child data is `childNodes`, since `children(name)` is the method
        if (Array.isArray(node.children)) node.childNodes = node.children;
        const all = (node.childNodes || []).map((kid) => kid.__wrap ? kid : wrap("Node", kid));
        node.childNodes = all;
        node.children = (name) => all.filter((kid) => name == null || kid.name === name);
        node.attribute = (name) => node.attributes[name] ?? null;
        node.first = (name) => all.find((kid) => name == null || kid.name === name) || null;
        node.hasChild = (name) => all.some((kid) => kid.name === name);
        node.toString = () => node.xml || "";
        return node;
    };
    const makeNode = (data) => wrap("Node", data);
    // a rule or tree that a screen shows with `toString()`
    wrappers.Text = (data) => Object.assign(data, { toString: () => data.text || "" });
    const makeText = (text) => wrap("Text", { text: text });

    // `ec.context.putAll(map)`: each entry on the stack by name
    ec.context.putAll = (map) => { for (const [key, value] of Object.entries(map || {})) globalThis[key] = value; };

    // ---- cache ----
    ec.cache.cacheExists = host.cacheExists;
    ec.cache.clearAllCaches = host.clearAllCaches;
    ec.cache.getAllCachesInfo = host.getAllCachesInfo;
    ec.cache.makeElementInfoList = host.makeElementInfoList;
    ec.cache.getLocalCache = ec.cache.getCache;
    ec.cache.getDistributedCache = ec.cache.getCache;

    // ---- entity ----
    ec.entity.isEntityDefined = host.isEntityDefined;
    ec.entity.getAllEntityNames = host.getAllEntityNames;
    ec.entity.getAllEntitiesInfo = host.getAllEntitiesInfo;
    ec.entity.getAllEntityInfo = host.getAllEntityInfo;
    ec.entity.getDatasourceGroupNames = host.getDatasourceGroupNames;
    ec.entity.getEecaRuleCount = host.getEecaRuleCount;
    // Query statistics are a conf flag, off by default in Java (entity-facade@query-stats); the port does not keep them.
    ec.entity.getQueryStatsList = () => [];
    ec.entity.clearQueryStats = () => {};
    ec.entity.runSqlStatement = host.runSqlStatement;
    ec.entity.eecaRulesByEntityName = {
        get: (entityName) => host.eecaRuleTexts(entityName).map((text) => ({ eecaNode: makeText(text) }))
    };
    // `RelationshipInfo`, with the calls of the auto screen menu
    wrappers.Relationship = (rel) => Object.assign(rel, {
        riRelatedEntityName: () => rel.relatedEntityName,
        riPrettyName: () => rel.prettyName,
        getTargetParameterMap: (values) => {
            const target = {};
            for (const [field, related] of Object.entries(rel.keyMap || {})) {
                const value = values == null ? null : (typeof values.get === "function" ? values.get(field) : values[field]);
                if (value != null && value !== "") target[related] = value;
            }
            return target;
        }
    });
    wrappers.EntityDefinition = function (ed) {
        const fieldByName = {};
        for (const field of ed.entityInfo.allFieldInfoArray) fieldByName[field.name] = field;
        const relationships = ed.relationshipsInfo;
        Object.assign(ed, {
            getFullEntityName: () => ed.fullEntityName,
            getPkFieldNames: () => ed.pkFieldNames.slice(),
            getAllFieldNames: () => ed.allFieldNames.slice(),
            getFieldNames: (pks, nonPks) => ed.entityInfo.allFieldInfoArray.filter((f) => (pks && f.isPk) || (nonPks && !f.isPk)).map((f) => f.name),
            isField: (name) => fieldByName[name] != null,
            isPkField: (name) => fieldByName[name] != null && fieldByName[name].isPk,
            getFieldInfo: (name) => fieldByName[name] || null,
            getColumnName: (name) => fieldByName[name] ? fieldByName[name].columnName : null,
            getPrettyName: (title, baseName) => host.prettyName(ed.entityName, title || null, baseName || null),
            getPrimaryKeys: (values) => {
                const pks = {};
                for (const name of ed.pkFieldNames) {
                    const value = values == null ? null : (typeof values.get === "function" ? values.get(name) : values[name]);
                    if (value != null && value !== "") pks[name] = value;
                }
                return pks;
            },
            getRelationshipsInfo: (dependentsOnly) => relationships.filter((rel) => !dependentsOnly || rel.dependent),
            getRelationshipInfo: (name) => relationships.find((rel) => rel.relationshipName === name || rel.shortAlias === name ||
                (rel.title == null && rel.relatedEntityName.endsWith("." + name))) || null,
            getMasterDefinitionMap: () => Object.fromEntries(ed.masterNames.map((name) => [name, { name: name }])),
            getEntityNode: () => ed.entityNode,
            getDependentsTree: () => Object.assign(makeText(ed.dependentsText), { allDescendants: ed.allDescendants })
        });
        return ed;
    };
    ec.entity.getEntityDefinition = function (entityName) {
        const info = host.entityDefinitionInfo(entityName);
        if (info == null) return null;
        for (const field of info.fields) field.fieldNode = makeNode({ name: "field", attributes: { name: field.name, type: field.type } });
        for (const rel of info.relationships) wrap("Relationship", rel);
        return wrap("EntityDefinition", {
            fullEntityName: info.fullEntityName, entityName: info.entityName, isViewEntity: info.isViewEntity,
            tableName: info.tableName, groupName: info.groupName, pkFieldNames: info.pkFieldNames,
            allFieldNames: info.allFieldNames, relationshipsInfo: info.relationships, masterNames: info.masterNames,
            entityNode: makeNode({ name: "entity", attributes: info.entityAttributes }),
            dependentsText: info.dependentsText, allDescendants: info.allDescendants,
            prettyName: host.prettyName(info.entityName, null, null),
            entityInfo: {
                internalEntityName: info.entityName, shortAlias: info.shortAlias, fullTableName: info.tableName,
                needsEncrypt: info.needsEncrypt, needsAuditLog: info.needsAuditLog, createOnly: info.createOnly,
                allFieldInfoArray: info.fields, allFieldInfoList: info.fields
            }
        });
    };
    // The model loads the reverse relationships with the definitions, so there is nothing to create.
    ec.entity.createAllAutoReverseManyRelationships = () => {};
    // `getAllEntityRelatedFields`: the fields of an entity and of each related entity, with whether a data view has them
    ec.entity.getAllEntityRelatedFields = function (entityName, orderByField, dbViewEntityName) {
        const ed = entityName ? ec.entity.getEntityDefinition(entityName) : null;
        if (ed == null) return [];
        const alias = (entityAlias, fieldName) => dbViewEntityName ? ec.entity.find("moqui.entity.view.DbViewEntityAlias")
            .condition({dbViewEntityName: dbViewEntityName, entityAlias: entityAlias, fieldName: fieldName}).one() : null;
        const out = [];
        for (const field of ed.entityInfo.allFieldInfoArray) {
            const aliasVal = alias("MASTER", field.name);
            out.push({entityName: ed.fullEntityName, fieldName: field.name, type: field.type, cardinality: "one",
                inDbView: aliasVal != null, functionName: aliasVal?.functionName ?? null});
        }
        for (const rel of ed.getRelationshipsInfo(false)) {
            const red = ec.entity.getEntityDefinition(rel.relatedEntityName);
            if (red == null) continue;
            const member = dbViewEntityName ? ec.entity.find("moqui.entity.view.DbViewEntityMember")
                .condition({dbViewEntityName: dbViewEntityName, entityName: red.fullEntityName}).list()[0] : null;
            for (const field of red.entityInfo.allFieldInfoArray) {
                const aliasVal = member ? alias(member.entityAlias, field.name) : null;
                out.push({entityName: rel.relatedEntityName, fieldName: field.name, type: field.type, cardinality: rel.type,
                    title: rel.title, inDbView: aliasVal != null, functionName: aliasVal?.functionName ?? null});
            }
        }
        if (orderByField) orderMapList(out, [orderByField]);
        return out;
    };
    ec.entity.makeDataWriter = function () {
        const ops = [];
        const w = {};
        const op = (name) => (value) => { ops.push([name, value]); return w; };
        for (const name of ["fileType", "entityName", "skipEntityName", "dependentLevels", "master", "prefix",
                "filterMap", "fromDate", "thruDate", "isoDateTime", "tableColumnNames"]) w[name] = op(name);
        w.entityNames = (names) => { for (const name of Array.from(names || [])) ops.push(["entityName", name]); return w; };
        w.skipEntityNames = (names) => { for (const name of Array.from(names || [])) ops.push(["skipEntityName", name]); return w; };
        w.allEntities = () => { ops.push(["allEntities", true]); return w; };
        w.dependentRecords = (flag) => { ops.push(["dependentLevels", flag ? 2 : 0]); return w; };
        w.orderBy = (fields) => { for (const field of Array.from(fields || [])) ops.push(["orderBy", field]); return w; };
        w.file = (filename) => host.writeData(ops, "file", filename, null).count;
        w.directory = (path) => host.writeData(ops, "directory", path, null).count;
        w.zipFile = (within, zipFilename) => host.writeData(ops, "zipFile", within, zipFilename).count;
        w.zipDirectory = (within, zipFilename) => {
            if (typeof zipFilename !== "string") throw new Error("zipDirectory to a stream is not available; write a zip file and send it as a resource");
            return host.writeData(ops, "zipDirectory", within, zipFilename).count;
        };
        // Java's writer(Writer): the text comes back, and the caller sends it.
        w.text = () => host.writeData(ops, "text", null, null).text;
        return w;
    };
    ec.entity.makeDataLoader = function () {
        const options = { dataTypes: [], componentNames: [] };
        const l = {};
        for (const name of ["location", "xmlText", "csvText", "jsonText", "dummyFks", "useTryInsert", "onlyCreate",
                "disableEntityEca", "disableAuditLog", "disableFkCreate", "disableDataFeed", "transactionTimeout",
                "messageNoActionFiles", "restrictSensitiveEntities", "csvEntityName"])
            l[name] = (value) => { options[name] = value; return l; };
        l.dataTypes = (types) => { options.dataTypes.push(...Array.from(types || [])); return l; };
        l.componentNameList = (names) => { options.componentNames.push(...Array.from(names || [])); return l; };
        l.load = (messages) => {
            const result = host.loadData(options);
            if (Array.isArray(messages)) messages.push(...result.messages);
            return result.records;
        };
        return l;
    };

    // ---- service ----
    ec.service.getKnownServiceNames = host.getKnownServiceNames;
    Object.defineProperty(ec.service, "knownServiceNames", { get: host.getKnownServiceNames });
    ec.service.getAllServiceInfo = host.getAllServiceInfo;
    ec.service.getSecaRuleCount = host.getSecaRuleCount;
    ec.service.secaRulesByServiceName = {
        get: (serviceName) => { const texts = host.secaRuleTexts(serviceName); return texts.length ? texts.map(makeText) : null; }
    };
    ec.service.getServiceDefinition = function (serviceName) {
        const info = host.serviceDefinitionInfo(serviceName);
        if (info == null) return null;
        info.serviceNode = makeNode(host.parseNode(info.serviceXml));
        return info;
    };
    wrappers.JobRunner = (info) => Object.assign(info, {
        getLastExecuteTime: () => info.lastExecuteTime, getExecuteCount: () => info.executeCount,
        getTotalJobsRun: () => info.totalJobsRun, getLastJobsActive: () => info.lastJobsActive,
        getLastJobsPaused: () => info.lastJobsPaused
    });
    ec.service.getJobRunner = () => wrap("JobRunner", host.jobRunnerInfo());
    // One pool runs service calls and jobs in this port, so the worker pool and the job pool are the same.
    wrappers.WorkerPool = (info) => Object.assign(info, {
        getActiveCount: () => info.activeCount, getPoolSize: () => info.poolSize, getMaximumPoolSize: () => info.maximumPoolSize,
        getCorePoolSize: () => info.poolSize,
        queue: { size: () => info.queueSize, remainingCapacity: () => info.remainingCapacity }
    });
    const poolInfo = () => {
        const info = host.workerPoolInfo();
        return wrap("WorkerPool", {
            activeCount: info.active, poolSize: info.workers, maximumPoolSize: info.workers, queueSize: info.queued,
            remainingCapacity: Math.max(info.queueCap - info.queued, 0)
        });
    };
    Object.defineProperty(ec.service, "jobWorkerPool", { get: poolInfo });

    // ---- screen ----
    ec.screen.getAllRootScreenLocations = host.getAllRootScreenLocations;
    wrappers.ScreenInfo = (info) => Object.assign(info, { getIndentedName: () => info.indentedName });
    ec.screen.getScreenInfoList = function (rootLocation, levels) {
        return host.getScreenInfoList(rootLocation, levels).map((info) =>
            wrap("ScreenInfo", Object.assign(info, { indentedName: "- ".repeat(info.level) + " " + info.name })));
    };

    // ---- resources: the parts of ResourceReference the tools screens and services call ----
    wrappers.Resource = (ref) => Object.assign(ref, {
        getLocation: () => ref.location, getFileName: () => ref.fileName, getExists: () => ref.exists,
        isDirectory: () => ref.directory, isFile: () => ref.file, getSize: () => ref.size,
        isText: () => ref.text, isBinary: () => ref.binary, getContentType: () => ref.contentType,
        getLastModified: () => ref.lastModified, length: () => ref.size || 0, getName: () => ref.fileName,
        getDirectoryEntries: () => (ref.children || []).map((child) => wrap("Resource", child)),
        getText: () => host.resourceText(ref.location),
        putText: (text) => host.resourcePutText(ref.location, String(text)),
        putBytes: (bytes) => host.resourcePutBytes(ref.location, bytes),
        delete: () => host.resourceDelete(ref.location)
    });
    const makeRef = (location) => wrap("Resource", host.resourceInfo(location));
    ec.resource.getLocationReference = (location) => makeRef(String(location));
    ec.resource.getLocationText = (location) => host.resourceText(String(location));

    // ---- forms: the field names a form has, for the DB form response services ----
    ec.screen.getFormNode = function (location) {
        location = String(location);
        if (location.startsWith("DbForm#")) {
            const fields = ec.entity.find("moqui.screen.form.DbFormField").condition("formId", location.substring(7))
                .orderBy("layoutSequenceNum").list();
            return makeNode({ name: "form-single", attributes: {},
                children: fields.map((field) => ({ name: "field", attributes: { name: field.fieldName } })) });
        }
        return makeNode(host.parseNode(host.formXml(location)));
    };
    ec.service.isLastRunBeforeLastSchedule = host.isLastRunBeforeLastSchedule;
    ec.service.restApi = { getFreshRootResources: host.restRootResources, getRootResources: host.restRootResources };
    ec.entity.getAllEntityNamesWithMaster = host.getAllEntityNamesWithMaster;

    // `ScreenUrlInfo.isPermitted(ec, null, AUTHZA_ALL)` as a check of one artifact: the screen, the type, the action.
    ec.artifactExecution = { isPermitted: host.isPermitted, getFindFiltersForUser: host.getFindFiltersForUser };

    // `Locale.getAvailableLocales()` with English display names. Java lists every JVM locale; the port has no locale
    // database, so this is a fixed list of the common ones.
    const localeNames = { ar: "Arabic", cs: "Czech", da: "Danish", de: "German", de_AT: "German (Austria)",
        de_CH: "German (Switzerland)", de_DE: "German (Germany)", el: "Greek", en: "English", en_AU: "English (Australia)",
        en_CA: "English (Canada)", en_GB: "English (United Kingdom)", en_IN: "English (India)", en_US: "English (United States)",
        es: "Spanish", es_ES: "Spanish (Spain)", es_MX: "Spanish (Mexico)", fi: "Finnish", fr: "French", fr_CA: "French (Canada)",
        fr_FR: "French (France)", he: "Hebrew", hi: "Hindi", hu: "Hungarian", it: "Italian", ja: "Japanese", ko: "Korean",
        nl: "Dutch", no: "Norwegian", pl: "Polish", pt: "Portuguese", pt_BR: "Portuguese (Brazil)", ro: "Romanian",
        ru: "Russian", sv: "Swedish", th: "Thai", tr: "Turkish", uk: "Ukrainian", vi: "Vietnamese", zh: "Chinese",
        zh_CN: "Chinese (China)", zh_TW: "Chinese (Taiwan)" };
    ec.l10n.getAvailableTimeZones = host.getAvailableTimeZones;
    // `formatFieldString(entityName, fieldName, text)`: Java converts the text to the field's type and formats it with
    // the user's locale; the audit log keeps the text, so it is shown as stored here.
    ec.entity.formatFieldString = (entityName, fieldName, text) => text == null ? "" : String(text);
    ec.l10n.getAvailableLocales = () => Object.entries(localeNames).map(([locale, name]) => ({ locale: locale, name: name }));

    // ---- user and messages ----
    // `getPeriodRange(baseName, inputs)`: from `{base}_period`, `_poffset`, and `_pdate`, else `_from` and `_thru`.
    // `getPeriodRange(period, poffset, pdate)`, Java's other form: the range itself, from a date (epoch millis or
    // yyyy-MM-dd) or today.
    ec.user.getPeriodRange = function (baseName, inputs, pdate) {
        if (arguments.length >= 3 || (inputs != null && typeof inputs !== "object"))
            return host.periodRange(String(baseName), String(inputs ?? ""), String(pdate ?? ""));
        inputs = inputs || {};
        const period = inputs[baseName + "_period"];
        if (period) return host.periodRange(String(period), String(inputs[baseName + "_poffset"] ?? ""), String(inputs[baseName + "_pdate"] ?? ""));
        return [inputs[baseName + "_from"] || null, inputs[baseName + "_thru"] || null];
    };
    Object.defineProperty(ec.message, "messageInfos", { get: host.messageInfos });

    // ---- factory and web ----
    ec.factory.getStatusMap = host.getStatusMap;
    ec.factory.getComponentInfoList = host.getComponentInfoList;
    ec.factory.getRuntimePath = host.getRuntimePath;
    Object.defineProperty(ec.factory, "workerPool", { get: poolInfo });
    Object.defineProperty(ec.factory, "versionMap", { get: () => ({ moqui: ec.factory.moquiVersion }) });
    ec.factory.getThreadList = host.getThreadList;
    ec.web.getRequestDetails = host.getRequestDetails;
    Object.defineProperty(ec.web, "secureRequestParameters", { get: host.secureRequestParameters });
    su.camelCaseToPretty = host.camelCaseToPretty;
    su.prettyToCamelCase = host.prettyToCamelCase;
    // `new URI(null, null, filename, null).toASCIIString()`: a path with the characters a URI does not allow escaped
    su.encodeAsciiFilename = (filename) => encodeURI(String(filename ?? ""));
    // Entities the generic tools (auto screens, data edit, data import) do not take from a request entity name
    const restrictedKeys = new Set(["userloginkey", "usergroupmember", "usergrouppermission", "userpermission",
        "artifactauthz", "artifactgroup", "artifactgroupmember", "artifacttarpit", "usergrouppermissions",
        "userpermissions", "artifactgroups", "userauthcfactor", "userpasswordhistory", "emailserver", "systemmessageremote"]);
    wu.isRestrictedGenericEntity = function (entityName) {
        if (entityName == null || entityName === "") return false;
        let name = String(entityName).trim();
        const dot = name.lastIndexOf(".");
        if (dot >= 0 && dot < name.length - 1) name = name.substring(dot + 1);
        return restrictedKeys.has(name.toLowerCase());
    };
})"#;

fn js_list<'js>(ctx: &Ctx<'js>, rows: Vec<indexmap::IndexMap<String, Value>>) -> rquickjs::Result<JsValue<'js>> {
    host_to_js(ctx, Value::List(rows.into_iter().map(Value::Map).collect()))
}

fn js_texts<'js>(ctx: &Ctx<'js>, texts: Vec<String>) -> rquickjs::Result<JsValue<'js>> {
    host_to_js(ctx, Value::List(texts.into_iter().map(Value::String).collect()))
}

fn factory_of(ctx: &Ctx<'_>) -> rquickjs::Result<Arc<crate::context::ExecutionContextFactory>> {
    with_ec(ctx, |ec| Arc::clone(ec.factory()))
}

fn opt_text(value: &Opt<JsValue<'_>>) -> String {
    match value.0.as_ref() {
        Some(value) if value.is_string() => value
            .as_string()
            .and_then(|text| text.to_string().ok())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn opt_flag(value: &Opt<JsValue<'_>>) -> bool {
    value.0.as_ref().is_some_and(js_truthy)
}

fn opt_count(value: &Opt<JsValue<'_>>, default: usize) -> usize {
    value
        .0
        .as_ref()
        .and_then(JsValue::as_number)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map_or(default, |number| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Checked finite and not negative.
            let count = number as usize;
            count
        })
}

// ---- cache ----

fn cache_info_row(info: &moqui_lib::cache::CacheInfo) -> indexmap::IndexMap<String, Value> {
    let number = |value: u64| Value::I64(i64::try_from(value).unwrap_or(i64::MAX));
    let mut row = indexmap::IndexMap::new();
    row.insert("name".to_string(), Value::string(&info.name));
    row.insert("expireTimeIdle".to_string(), Value::I64(info.expire_idle_ms / 1000));
    row.insert("expireTimeLive".to_string(), Value::I64(info.expire_live_ms / 1000));
    row.insert("maxElements".to_string(), number(info.max_entries as u64));
    row.insert("evictionStrategy".to_string(), Value::string("LRU"));
    row.insert("size".to_string(), number(info.size as u64));
    row.insert("getCount".to_string(), number(info.stats.gets));
    row.insert("putCount".to_string(), number(info.stats.puts));
    row.insert("hitCount".to_string(), number(info.stats.hits));
    row.insert("missCountTotal".to_string(), number(info.stats.misses));
    row.insert("evictionCount".to_string(), number(info.stats.evictions));
    row.insert("removeCount".to_string(), number(info.stats.removals));
    row.insert("expireCount".to_string(), number(info.stats.expires));
    row
}

/// Every cache the admin screen lists: the conf caches, the screen caches, and the entity list caches.
fn all_cache_infos(ctx: &Ctx<'_>) -> rquickjs::Result<Vec<moqui_lib::cache::CacheInfo>> {
    let factory = factory_of(ctx)?;
    let mut infos = factory
        .cache()
        .infos()
        .map_err(|err| Exception::throw_message(ctx, err.message()))?;
    infos.extend(crate::screen::ScreenFacade::of(&factory).cache_infos());
    if let Some(facade) = factory.entity() {
        infos.extend(crate::entity::cache::list_cache_infos(&facade));
    }
    Ok(infos)
}

fn cache_exists(ctx: Ctx<'_>, name: String) -> rquickjs::Result<bool> {
    host_guard(&ctx, || Ok(all_cache_infos(&ctx)?.iter().any(|info| info.name == name)))
}

/// `clearAllCaches`: the conf caches, the screen definitions, and the entity lists.
fn clear_all_caches(ctx: Ctx<'_>) -> rquickjs::Result<()> {
    host_guard(&ctx, || {
        let factory = factory_of(&ctx)?;
        factory
            .cache()
            .clear_all()
            .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        crate::screen::ScreenFacade::of(&factory).clear();
        if let Some(facade) = factory.entity() {
            crate::entity::cache::clear_all_list_caches(&facade).map_err(|err| throw_lib(&ctx, &err))?;
        }
        Ok(())
    })
}

/// Clear a cache that is not a conf cache: the screen definitions or an entity list. `false` when there is none.
pub(super) fn clear_other_cache(name: &str) -> Result<bool, moqui_lib::error::Error> {
    let Some(factory) = ExecutionContext::with_current(|ec| Arc::clone(ec.factory())) else {
        return Ok(false);
    };
    if crate::screen::ScreenFacade::of(&factory).clear_named(name) {
        return Ok(true);
    }
    match factory.entity() {
        Some(facade) => crate::entity::cache::clear_list_cache(&facade, name),
        None => Ok(false),
    }
}

fn get_all_caches_info<'js>(
    ctx: Ctx<'js>,
    order_by: Opt<JsValue<'js>>,
    filter: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let filter = opt_text(&filter);
        let mut rows: Vec<_> = all_cache_infos(&ctx)?
            .iter()
            .filter(|info| crate::entity::info::name_matches(&info.name, &filter))
            .map(cache_info_row)
            .collect();
        order_maps(&mut rows, &opt_text(&order_by));
        js_list(&ctx, rows)
    })
}

/// Order maps by one field, `-` descending, numbers by value and the rest as text.
fn order_maps(rows: &mut [indexmap::IndexMap<String, Value>], order_by: &str) {
    let descending = order_by.starts_with('-');
    let field = order_by.trim_start_matches(['-', '+', '^']);
    if field.is_empty() {
        return;
    }
    rows.sort_by(|left, right| {
        let ordering = match (left.get(field), right.get(field)) {
            (Some(Value::I64(left)), Some(Value::I64(right))) => left.cmp(right),
            (left, right) => {
                let text = |value: Option<&Value>| value.map(moqui_lib::object::to_plain_string).unwrap_or_default();
                text(left).cmp(&text(right))
            }
        };
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    });
}

fn make_element_info_list<'js>(
    ctx: Ctx<'js>,
    name: String,
    order_by: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let factory = factory_of(&ctx)?;
        let found = factory
            .cache()
            .get(&name)
            .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        let stamp = |millis: i64| jiff::Timestamp::from_millisecond(millis).map_or(Value::Null, Value::Timestamp);
        let mut rows: Vec<_> = found
            .map(|cache| cache.entry_infos())
            .unwrap_or_default()
            .into_iter()
            .map(|entry| {
                let mut row = indexmap::IndexMap::new();
                row.insert("key".to_string(), Value::string(entry.key));
                row.insert("value".to_string(), entry.value);
                row.insert(
                    "hitCount".to_string(),
                    Value::I64(i64::try_from(entry.hit_count).unwrap_or(i64::MAX)),
                );
                row.insert("creationTime".to_string(), stamp(entry.created));
                if entry.updated > 0 {
                    row.insert("lastUpdateTime".to_string(), stamp(entry.updated));
                }
                if entry.accessed > 0 {
                    row.insert("lastAccessTime".to_string(), stamp(entry.accessed));
                }
                row
            })
            .collect();
        order_maps(&mut rows, &opt_text(&order_by));
        js_list(&ctx, rows)
    })
}

// ---- entity ----

fn is_entity_defined(ctx: Ctx<'_>, name: Opt<JsValue<'_>>) -> rquickjs::Result<bool> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        Ok(crate::entity::info::is_entity_defined(&facade, &opt_text(&name)))
    })
}

fn get_all_entity_names<'js>(ctx: Ctx<'js>, filter: Opt<JsValue<'js>>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        js_texts(&ctx, crate::entity::info::all_entity_names(&facade, &opt_text(&filter)))
    })
}

fn get_all_entities_info<'js>(
    ctx: Ctx<'js>,
    order_by: Opt<JsValue<'js>>,
    filter: Opt<JsValue<'js>>,
    master_only: Opt<JsValue<'js>>,
    exclude_views: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let rows = crate::entity::info::all_entities_info(
            &facade,
            &opt_text(&order_by),
            &opt_text(&filter),
            opt_flag(&master_only),
            opt_flag(&exclude_views),
        );
        js_list(&ctx, rows)
    })
}

fn get_all_entity_info<'js>(
    ctx: Ctx<'js>,
    levels: Opt<JsValue<'js>>,
    exclude_views: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let rows = crate::entity::info::all_entity_info(&facade, opt_count(&levels, 2), opt_flag(&exclude_views));
        js_list(&ctx, rows)
    })
}

fn entity_definition_info<'js>(ctx: Ctx<'js>, name: Opt<JsValue<'js>>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        match crate::entity::info::definition_info(&facade, &opt_text(&name)) {
            Some(info) => host_to_js(&ctx, Value::Map(info)),
            None => Null.into_js(&ctx),
        }
    })
}

fn pretty_name(
    ctx: Ctx<'_>,
    entity_name: String,
    title: Opt<JsValue<'_>>,
    base_name: Opt<JsValue<'_>>,
) -> rquickjs::Result<String> {
    host_guard(&ctx, || {
        let title = opt_text(&title);
        let base = opt_text(&base_name);
        Ok(crate::entity::info::pretty_name(
            &entity_name,
            Some(title.as_str()).filter(|text| !text.is_empty()),
            Some(base.as_str()).filter(|text| !text.is_empty()),
        ))
    })
}

fn get_datasource_group_names<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        js_texts(&ctx, crate::entity::info::datasource_group_names(&facade))
    })
}

fn get_eeca_rule_count(ctx: Ctx<'_>) -> rquickjs::Result<i32> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        Ok(i32::try_from(crate::entity::load_feed::eca_rule_count(&facade)).unwrap_or(i32::MAX))
    })
}

/// The EECA rules of an entity, each as the XML of its rule.
fn eeca_rule_texts<'js>(ctx: Ctx<'js>, entity_name: String) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let texts = crate::entity::load_feed::eca_rules(&facade, &entity_name)
            .into_iter()
            .map(|rule| {
                let on = |flag: bool, name: &str| {
                    if flag {
                        format!(" {name}=\"true\"")
                    } else {
                        String::new()
                    }
                };
                format!(
                    "<eeca id=\"{}\" entity=\"{}\"{}{}{}{}>{}{}</eeca>",
                    rule.id,
                    rule.entity_name,
                    on(rule.on_create, "on-create"),
                    on(rule.on_update, "on-update"),
                    on(rule.on_delete, "on-delete"),
                    on(rule.run_before, "run-before"),
                    rule.condition,
                    rule.actions
                )
            })
            .collect();
        js_texts(&ctx, texts)
    })
}

/// The SQL runner's statement: `{columns, records, limitReached, rowsAffected}`, values formatted as text.
fn run_sql_statement<'js>(
    ctx: Ctx<'js>,
    group: String,
    sql: String,
    limit: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let result = crate::entity::info::run_sql(&facade, &group, &sql, opt_count(&limit, 500))
            .map_err(|err| throw_lib(&ctx, &err))?;
        let mut map = indexmap::IndexMap::new();
        map.insert(
            "columns".to_string(),
            Value::List(result.columns.into_iter().map(Value::String).collect()),
        );
        let records = result
            .records
            .into_iter()
            .map(|row| {
                Value::List(
                    row.into_iter()
                        .map(|value| match value {
                            Value::Bytes(bytes) => Value::string(format!("({} bytes)", bytes.len())),
                            other => other,
                        })
                        .collect(),
                )
            })
            .collect();
        map.insert("records".to_string(), Value::List(records));
        map.insert("limitReached".to_string(), Value::Bool(result.limit_reached));
        map.insert(
            "rowsAffected".to_string(),
            result.rows_affected.map_or(Value::Null, |count| {
                Value::I64(i64::try_from(count).unwrap_or(i64::MAX))
            }),
        );
        host_to_js(&ctx, Value::Map(map))
    })
}

/// The data writer: replay the builder's calls, then write in `mode`. Returns `{count, text}`.
fn write_data<'js>(
    ctx: Ctx<'js>,
    ops: JsValue<'js>,
    mode: String,
    first: Opt<JsValue<'js>>,
    second: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let mut writer = crate::entity::EntityDataWriter::new(&facade);
        let Value::List(ops) = js_to_host(&ctx, &ops)? else {
            return Err(Exception::throw_message(&ctx, "data writer calls are a list"));
        };
        for op in ops {
            let Value::List(pair) = op else { continue };
            let name = pair.first().map(moqui_lib::object::to_plain_string).unwrap_or_default();
            let value = pair.get(1).cloned().unwrap_or(Value::Null);
            let text = moqui_lib::object::to_plain_string(&value);
            let flag = matches!(value, Value::Bool(true)) || text == "true";
            match name.as_str() {
                "fileType" => {
                    writer.file_type(crate::entity::DataFileType::parse(&text).map_err(|err| throw_lib(&ctx, &err))?);
                }
                "entityName" => {
                    writer.entity_name(&text);
                }
                "skipEntityName" => {
                    writer.skip_entity_name(&text);
                }
                "allEntities" => {
                    writer.all_entities();
                }
                "dependentLevels" => {
                    writer.dependent_levels(text.parse().unwrap_or(0));
                }
                "master" => {
                    writer.master(&text);
                }
                "prefix" => {
                    writer.prefix(&text);
                }
                "filterMap" => {
                    if let Value::Map(map) = value {
                        writer.filter_map(map.into_iter().collect());
                    }
                }
                "orderBy" => {
                    writer.order_by(&text);
                }
                "fromDate" => {
                    writer.from_date(value);
                }
                "thruDate" => {
                    writer.thru_date(value);
                }
                "isoDateTime" => {
                    writer.iso_date_time(flag);
                }
                "tableColumnNames" => {
                    writer.table_column_names(flag);
                }
                _ => {}
            }
        }
        let first = opt_text(&first);
        let second = opt_text(&second);
        let mut out = indexmap::IndexMap::new();
        let lib = |result: Result<u64, moqui_lib::error::Error>| result.map_err(|err| throw_lib(&ctx, &err));
        let count = match mode.as_str() {
            "file" => lib(writer.file(&first))?,
            "directory" => lib(writer.directory(&first))?,
            "zipFile" => lib(writer.zip_file(&first, &second))?,
            "zipDirectory" => lib(writer.zip_directory(&first, &second))?,
            _ => {
                let (text, count) = writer.text().map_err(|err| throw_lib(&ctx, &err))?;
                out.insert("text".to_string(), Value::String(text));
                count
            }
        };
        out.insert(
            "count".to_string(),
            Value::I64(i64::try_from(count).unwrap_or(i64::MAX)),
        );
        host_to_js(&ctx, Value::Map(out))
    })
}

/// The data loader: `{location | xmlText | csvText | jsonText, dataTypes, componentNames, ...}` to `{records,
/// messages}`. With data types or components and no location it loads the conf data, as `ec.entity.makeDataLoader()
/// .dataTypes(...).load()` does. CSV and JSON text go through a file in the runtime's `tmp` directory.
fn load_data<'js>(ctx: Ctx<'js>, options: JsValue<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let Value::Map(options) = js_to_host(&ctx, &options)? else {
            return Err(Exception::throw_message(&ctx, "the data loader takes an options map"));
        };
        let text = |name: &str| {
            options
                .get(name)
                .map(moqui_lib::object::to_plain_string)
                .unwrap_or_default()
        };
        let flag = |name: &str| matches!(options.get(name), Some(Value::Bool(true))) || text(name) == "true";
        let list = |name: &str| match options.get(name) {
            Some(Value::List(items)) => items.iter().map(moqui_lib::object::to_plain_string).collect(),
            _ => Vec::new(),
        };
        let load_options = crate::entity::load_feed::LoadOptions {
            data_types: list("dataTypes"),
            components: list("componentNames"),
            only_create: flag("onlyCreate"),
            use_try_insert: flag("useTryInsert"),
            dummy_fks: flag("dummyFks"),
            disable_fk_create: flag("disableFkCreate"),
            timeout: text("transactionTimeout")
                .parse::<u64>()
                .ok()
                .map(std::time::Duration::from_secs),
            restrict_sensitive_entities: flag("restrictSensitiveEntities"),
        };
        let before = facade.flags();
        facade.set_disable_entity_eca(flag("disableEntityEca") || before.disable_entity_eca);
        facade.set_disable_audit_log(flag("disableAuditLog") || before.disable_audit_log);
        facade.set_disable_data_feed(flag("disableDataFeed") || before.disable_data_feed);
        let factory = factory_of(&ctx)?;
        let loaded = load_source(&facade, &factory, &text, &load_options);
        facade.set_disable_entity_eca(before.disable_entity_eca);
        facade.set_disable_audit_log(before.disable_audit_log);
        facade.set_disable_data_feed(before.disable_data_feed);
        let (records, messages) = loaded.map_err(|err| throw_lib(&ctx, &err))?;
        let mut out = indexmap::IndexMap::new();
        out.insert(
            "records".to_string(),
            Value::I64(i64::try_from(records).unwrap_or(i64::MAX)),
        );
        out.insert(
            "messages".to_string(),
            Value::List(messages.into_iter().map(Value::String).collect()),
        );
        host_to_js(&ctx, Value::Map(out))
    })
}

fn load_source(
    facade: &crate::entity::EntityFacade,
    factory: &crate::context::ExecutionContextFactory,
    text: &dyn Fn(&str) -> String,
    options: &crate::entity::load_feed::LoadOptions,
) -> Result<(u64, Vec<String>), moqui_lib::error::Error> {
    use crate::entity::load_feed::{conf_data_locations, load_entity_data, load_entity_xml_text};
    let location = text("location");
    if !location.is_empty() {
        let result = load_entity_data(facade, &location, options)?;
        return Ok((
            result.records,
            vec![format!("Loaded {} records from {location}", result.records)],
        ));
    }
    let xml = text("xmlText");
    if !xml.is_empty() {
        let result = load_entity_xml_text(facade, "xmlText", &xml, options)?;
        return Ok((result.records, Vec::new()));
    }
    for (name, extension) in [("csvText", "csv"), ("jsonText", "json")] {
        let body = text(name);
        if body.is_empty() {
            continue;
        }
        let dir = factory.runtime_dir().join("tmp");
        std::fs::create_dir_all(&dir).map_err(|err| {
            moqui_lib::error::Error::new(format!("could not create {}", dir.display())).with_source(err)
        })?;
        let path = dir.join(format!("data-load-{}.{extension}", crate::entity::uuid_v4()?));
        std::fs::write(&path, body).map_err(|err| {
            moqui_lib::error::Error::new(format!("could not write {}", path.display())).with_source(err)
        })?;
        let result = load_entity_data(facade, &path.display().to_string(), options);
        let _ = std::fs::remove_file(&path);
        let result = result?;
        return Ok((result.records, Vec::new()));
    }
    if options.data_types.is_empty() && options.components.is_empty() {
        return Ok((0, Vec::new()));
    }
    let mut records = 0;
    let mut messages = Vec::new();
    for location in conf_data_locations(facade)? {
        let result = load_entity_data(facade, &location, options)?;
        if result.records > 0 {
            messages.push(format!("Loaded {} records from {location}", result.records));
        }
        records += result.records;
    }
    Ok((records, messages))
}

// ---- service ----

fn get_known_service_names<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || js_texts(&ctx, crate::service::info::known_service_names()))
}

fn get_all_service_info<'js>(ctx: Ctx<'js>, levels: Opt<JsValue<'js>>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let levels = opt_count(&levels, 2);
        let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
        for name in crate::service::info::known_service_names() {
            *counts
                .entry(crate::entity::info::name_prefix(&name, levels))
                .or_default() += 1;
        }
        let rows = counts
            .into_iter()
            .map(|(name, services)| {
                let mut row = indexmap::IndexMap::new();
                row.insert("name".to_string(), Value::String(name));
                row.insert("services".to_string(), Value::I64(services));
                row
            })
            .collect();
        js_list(&ctx, rows)
    })
}

fn get_seca_rule_count(ctx: Ctx<'_>) -> rquickjs::Result<i32> {
    host_guard(&ctx, || {
        Ok(i32::try_from(crate::service::info::seca_rule_count()).unwrap_or(i32::MAX))
    })
}

fn seca_rule_texts<'js>(ctx: Ctx<'js>, service: String) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || js_texts(&ctx, crate::service::info::seca_rule_texts(&service)))
}

fn service_definition_info<'js>(ctx: Ctx<'js>, name: Opt<JsValue<'js>>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        match crate::service::info::service_definition_info(&opt_text(&name)) {
            Some(info) => host_to_js(&ctx, Value::Map(info)),
            None => Null.into_js(&ctx),
        }
    })
}

/// An XML element as `{name, attributes, text, children, xml}`, for the node wrapper of the screens.
fn node_value(node: &MNode) -> Value {
    let mut map = indexmap::IndexMap::new();
    map.insert("name".to_string(), Value::string(node.name()));
    map.insert(
        "attributes".to_string(),
        Value::Map(
            node.attributes()
                .into_iter()
                .map(|(name, value)| (name, Value::String(value)))
                .collect(),
        ),
    );
    map.insert("text".to_string(), node.text().map_or(Value::Null, Value::String));
    map.insert(
        "children".to_string(),
        Value::List(node.children().iter().map(node_value).collect()),
    );
    map.insert("xml".to_string(), Value::string(node.write_string()));
    Value::Map(map)
}

fn parse_node<'js>(ctx: Ctx<'js>, xml: String) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || match MNode::parse_text("node", &xml) {
        Ok(node) => host_to_js(&ctx, node_value(&node)),
        Err(_) => Null.into_js(&ctx),
    })
}

fn job_runner_info<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let factory = factory_of(&ctx)?;
        let Some(stats) = factory.service_pools().runner_stats() else {
            return Null.into_js(&ctx);
        };
        let number = |value: u64| Value::I64(i64::try_from(value).unwrap_or(i64::MAX));
        let mut map = indexmap::IndexMap::new();
        map.insert(
            "lastExecuteTime".to_string(),
            jiff::Timestamp::from_millisecond(stats.last_execute_millis).map_or(Value::Null, Value::Timestamp),
        );
        map.insert("executeCount".to_string(), number(stats.execute_count));
        map.insert("totalJobsRun".to_string(), number(stats.total_jobs_run));
        map.insert("lastJobsActive".to_string(), number(stats.last_jobs_active as u64));
        map.insert("lastJobsPaused".to_string(), number(stats.last_jobs_paused as u64));
        host_to_js(&ctx, Value::Map(map))
    })
}

fn worker_pool_info<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let factory = factory_of(&ctx)?;
        let status = factory.service_pools().status();
        let number = |value: usize| Value::I64(i64::try_from(value).unwrap_or(i64::MAX));
        let mut map = indexmap::IndexMap::new();
        map.insert("workers".to_string(), number(status.workers));
        map.insert("active".to_string(), number(status.active));
        map.insert("queued".to_string(), number(status.queued));
        map.insert("queueCap".to_string(), number(status.queue_cap));
        host_to_js(&ctx, Value::Map(map))
    })
}

// ---- screen, factory, web ----

fn get_all_root_screen_locations<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let factory = factory_of(&ctx)?;
        js_texts(&ctx, crate::screen::info::all_root_screen_locations(&factory))
    })
}

fn get_screen_info_list<'js>(
    ctx: Ctx<'js>,
    root_location: String,
    levels: Opt<JsValue<'js>>,
) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let factory = factory_of(&ctx)?;
        let rows = crate::screen::info::screen_info_list(&factory, &root_location, opt_count(&levels, 2))
            .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        js_list(&ctx, rows)
    })
}

fn get_status_map<'js>(ctx: Ctx<'js>, include_sensitive: Opt<JsValue<'js>>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let factory = factory_of(&ctx)?;
        let map = crate::context::status::status_map(&factory, opt_flag(&include_sensitive));
        host_to_js(&ctx, Value::Map(map))
    })
}

fn get_component_info_list<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let factory = factory_of(&ctx)?;
        js_list(&ctx, crate::context::status::component_info_list(&factory))
    })
}

fn get_runtime_path(ctx: Ctx<'_>) -> rquickjs::Result<String> {
    host_guard(&ctx, || Ok(factory_of(&ctx)?.runtime_dir().display().to_string()))
}

fn get_thread_list<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || js_list(&ctx, crate::context::status::thread_list()))
}

fn secure_request_parameters<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let map = crate::web::secure_parameters().into_iter().collect();
        host_to_js(&ctx, Value::Map(map))
    })
}

fn get_request_details(ctx: Ctx<'_>) -> rquickjs::Result<String> {
    host_guard(&ctx, || Ok(crate::web::request_details()))
}

// ---- resources ----

fn resource_ref(ctx: &Ctx<'_>, location: &str) -> rquickjs::Result<crate::context::ResourceRef> {
    let found = with_ec(ctx, |ec| ec.factory().resource().get(location))?;
    found.map_err(|err| Exception::throw_message(ctx, err.message()))
}

fn resource_info_value(resource: &crate::context::ResourceRef, children: bool) -> Value {
    let location = resource.location().to_string();
    let file_name = location.rsplit('/').next().unwrap_or_default().to_string();
    let mut map = indexmap::IndexMap::new();
    map.insert("location".to_string(), Value::string(&location));
    map.insert("fileName".to_string(), Value::string(&file_name));
    map.insert("exists".to_string(), Value::Bool(resource.exists()));
    map.insert("directory".to_string(), Value::Bool(resource.is_directory()));
    map.insert("file".to_string(), Value::Bool(resource.is_file()));
    // `ResourceReference.getContentType`, `isText`, `isBinary`: from the file name's extension.
    let content_type = moqui_lib::resource::content_type_for_name(&file_name);
    map.insert(
        "contentType".to_string(),
        content_type.map_or(Value::Null, Value::string),
    );
    map.insert(
        "text".to_string(),
        Value::Bool(content_type.is_some_and(moqui_lib::resource::is_text_content_type)),
    );
    map.insert(
        "binary".to_string(),
        Value::Bool(content_type.is_some_and(moqui_lib::resource::is_binary_content_type)),
    );
    map.insert(
        "size".to_string(),
        resource
            .size()
            .map_or(Value::Null, |size| Value::I64(i64::try_from(size).unwrap_or(i64::MAX))),
    );
    map.insert(
        "lastModified".to_string(),
        resource.last_modified().map_or(Value::Null, Value::I64),
    );
    if children && resource.is_directory() {
        let entries = resource.children().unwrap_or_default();
        map.insert(
            "children".to_string(),
            Value::List(entries.iter().map(|child| resource_info_value(child, false)).collect()),
        );
    }
    Value::Map(map)
}

/// `{location, fileName, exists, directory, file, size, lastModified, children}`, or null for a location no scheme
/// resolves.
fn resource_info<'js>(ctx: Ctx<'js>, location: String) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        match with_ec(&ctx, |ec| ec.factory().resource().get(&location))? {
            Ok(resource) => host_to_js(&ctx, resource_info_value(&resource, true)),
            Err(_) => Null.into_js(&ctx),
        }
    })
}

fn resource_text(ctx: Ctx<'_>, location: String) -> rquickjs::Result<String> {
    host_guard(&ctx, || {
        resource_ref(&ctx, &location)?
            .read_text()
            .map_err(|err| Exception::throw_message(&ctx, err.message()))
    })
}

/// `putText`: a `dbresource://` row, or a file for the file-backed schemes.
fn resource_put_text(ctx: Ctx<'_>, location: String, text: String) -> rquickjs::Result<()> {
    put_resource_bytes(&ctx, &location, text.as_bytes())
}

/// `putBytes`: a `Uint8Array` or `ArrayBuffer`, written as `putText` writes.
fn resource_put_bytes<'js>(ctx: Ctx<'js>, location: String, bytes: JsValue<'js>) -> rquickjs::Result<()> {
    let Some(bytes) = bytes.as_object().and_then(super::convert::js_bytes) else {
        return Err(Exception::throw_message(
            &ctx,
            "putBytes needs a Uint8Array or an ArrayBuffer",
        ));
    };
    put_resource_bytes(&ctx, &location, &bytes)
}

fn put_resource_bytes(ctx: &Ctx<'_>, location: &str, bytes: &[u8]) -> rquickjs::Result<()> {
    host_guard(ctx, || {
        let written = with_ec(ctx, |ec| ec.factory().resource().write_bytes(location, bytes))?;
        written.map_err(|err| Exception::throw_message(ctx, err.message()))
    })
}

/// `delete` of a file-backed resource. `false` when there is no file there.
fn resource_delete(ctx: Ctx<'_>, location: String) -> rquickjs::Result<bool> {
    host_guard(&ctx, || {
        let resource = resource_ref(&ctx, &location)?;
        match resource.file_path() {
            Some(path) if path.is_file() => std::fs::remove_file(path)
                .map(|()| true)
                .map_err(|err| Exception::throw_message(&ctx, &format!("{location}: {err}"))),
            _ => Ok(false),
        }
    })
}

/// The XML of the form named after `#` in a screen location: `component://.../Screen.xml#FormName`.
fn form_xml(ctx: Ctx<'_>, location: String) -> rquickjs::Result<String> {
    host_guard(&ctx, || {
        let (screen, form) = location.split_once('#').unwrap_or((location.as_str(), ""));
        let factory = factory_of(&ctx)?;
        let screens = crate::screen::ScreenFacade::of(&factory);
        let def = screens
            .screen_def(&factory, screen)
            .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        let found = crate::screen::form::assemble(&screens, &factory, &def, form)
            .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        found
            .node()
            .map(|node| node.write_string())
            .map_err(|err| Exception::throw_message(&ctx, err.message()))
    })
}

/// `ScheduledJobRunner.isLastRunBeforeLastSchedule`: no last run, or the last run is before the latest time the cron
/// expression came due.
fn is_last_run_before_last_schedule(ctx: Ctx<'_>, cron: String, last_run: Opt<JsValue<'_>>) -> rquickjs::Result<bool> {
    host_guard(&ctx, || {
        let last = match last_run.0.as_ref() {
            Some(value) => {
                match moqui_lib::object::basic_convert(&js_to_host(&ctx, value)?, "java.sql.Timestamp").value {
                    Value::Timestamp(stamp) => Some(stamp.as_millisecond()),
                    _ => None,
                }
            }
            None => None,
        };
        crate::service::info::is_last_run_before_last_schedule(&cron, last)
            .map_err(|err| Exception::throw_message(&ctx, err.message()))
    })
}

/// Whether the user may do `action` on the artifact, checked as a push would check it, without pushing it.
fn is_permitted(ctx: Ctx<'_>, name: String, artifact_type: Opt<String>, action: Opt<String>) -> rquickjs::Result<bool> {
    host_guard(&ctx, || {
        let artifact_type = artifact_type.0.unwrap_or_else(|| "AT_XML_SCREEN".to_string());
        let action = action.0.unwrap_or_else(|| "view".to_string());
        Ok(crate::user::authorize(&artifact_type, &name, &action).is_ok())
    })
}

/// `getFindFiltersForUser(entityName)`: the `EntityFilter` records that apply to a find on the entity now, or null
/// when none do, as Java returns (an empty list would be true in a script condition).
fn get_find_filters_for_user<'js>(ctx: Ctx<'js>, entity_name: String) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let filters = crate::user::find_filters_for_user(&entity_name)
            .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        if filters.is_empty() {
            return Ok(JsValue::new_null(ctx.clone()));
        }
        let rows = filters
            .iter()
            .map(|filter| {
                filter
                    .fields()
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            })
            .collect();
        js_list(&ctx, rows)
    })
}

fn rest_root_resources<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let roots =
            crate::service::root_resource_infos().map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        host_to_js(&ctx, Value::List(roots))
    })
}

/// `getAllEntityNamesWithMaster`: the entities that have a master definition, sorted.
fn get_all_entity_names_with_master<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let facade = facade_of(&ctx)?;
        let mut names: Vec<String> = facade.with_model(|model| {
            model
                .entities()
                .iter()
                .filter(|entity| !entity.masters.is_empty())
                .map(|entity| entity.full_name.clone())
                .collect()
        });
        names.sort();
        js_texts(&ctx, names)
    })
}

fn period_range<'js>(ctx: Ctx<'js>, period: String, offset: String, basis: String) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let (from, thru) = crate::user::period_range(&period, &offset, &basis)
            .map_err(|err| Exception::throw_message(&ctx, err.message()))?;
        host_to_js(&ctx, Value::List(vec![from, thru]))
    })
}

/// `ec.message.messageInfos`: the messages as `{message, typeString}`. The port keeps no type with a message.
fn message_infos<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let messages = with_ec(&ctx, |ec| ec.message().messages().to_vec())?;
        let rows = messages
            .into_iter()
            .map(|message| {
                let mut row = indexmap::IndexMap::new();
                row.insert("message".to_string(), Value::String(message));
                row.insert("typeString".to_string(), Value::string("info"));
                row
            })
            .collect();
        js_list(&ctx, rows)
    })
}

/// `TimeZone.getAvailableIDs()`: the zone names of the time zone database, sorted.
fn get_available_time_zones<'js>(ctx: Ctx<'js>) -> rquickjs::Result<JsValue<'js>> {
    host_guard(&ctx, || {
        let mut names: Vec<String> = jiff::tz::db().available().map(|name| name.to_string()).collect();
        names.sort();
        js_texts(&ctx, names)
    })
}

fn camel_case_to_pretty(text: Opt<JsValue<'_>>) -> String {
    moqui_lib::string::camel_case_to_pretty(&opt_text(&text))
}

fn pretty_to_camel_case(text: Opt<JsValue<'_>>, first_upper: Opt<JsValue<'_>>) -> String {
    moqui_lib::string::pretty_to_camel_case(&opt_text(&text), opt_flag(&first_upper))
}

/// Add the tools calls to `ec` and the helper objects. `ec` is a global by now.
pub(super) fn install<'js>(
    ctx: &Ctx<'js>,
    string_utilities: &Object<'js>,
    collection_utilities: &Object<'js>,
    web_utilities: &Object<'js>,
) -> Result<(), Error> {
    let host = Object::new(ctx.clone()).map_err(js_error)?;
    macro_rules! method {
        ($name:literal, $function:expr) => {
            let function = caught(ctx, $name, Function::new(ctx.clone(), $function))?;
            caught(ctx, $name, host.set($name, function))?;
        };
    }
    method!("cacheExists", cache_exists);
    method!("clearAllCaches", clear_all_caches);
    method!("getAllCachesInfo", get_all_caches_info);
    method!("makeElementInfoList", make_element_info_list);
    method!("isEntityDefined", is_entity_defined);
    method!("getAllEntityNames", get_all_entity_names);
    method!("getAllEntitiesInfo", get_all_entities_info);
    method!("getAllEntityInfo", get_all_entity_info);
    method!("entityDefinitionInfo", entity_definition_info);
    method!("prettyName", pretty_name);
    method!("getDatasourceGroupNames", get_datasource_group_names);
    method!("getEecaRuleCount", get_eeca_rule_count);
    method!("eecaRuleTexts", eeca_rule_texts);
    method!("runSqlStatement", run_sql_statement);
    method!("writeData", write_data);
    method!("loadData", load_data);
    method!("getKnownServiceNames", get_known_service_names);
    method!("getAllServiceInfo", get_all_service_info);
    method!("getSecaRuleCount", get_seca_rule_count);
    method!("secaRuleTexts", seca_rule_texts);
    method!("serviceDefinitionInfo", service_definition_info);
    method!("parseNode", parse_node);
    method!("jobRunnerInfo", job_runner_info);
    method!("workerPoolInfo", worker_pool_info);
    method!("getAllRootScreenLocations", get_all_root_screen_locations);
    method!("getScreenInfoList", get_screen_info_list);
    method!("getStatusMap", get_status_map);
    method!("getComponentInfoList", get_component_info_list);
    method!("getRuntimePath", get_runtime_path);
    method!("getThreadList", get_thread_list);
    method!("getRequestDetails", get_request_details);
    method!("secureRequestParameters", secure_request_parameters);
    method!("camelCaseToPretty", camel_case_to_pretty);
    method!("getAvailableTimeZones", get_available_time_zones);
    method!("resourceInfo", resource_info);
    method!("restRootResources", rest_root_resources);
    method!("isPermitted", is_permitted);
    method!("getFindFiltersForUser", get_find_filters_for_user);
    method!("getAllEntityNamesWithMaster", get_all_entity_names_with_master);
    method!("periodRange", period_range);
    method!("messageInfos", message_infos);
    method!("resourceText", resource_text);
    method!("resourcePutText", resource_put_text);
    method!("resourcePutBytes", resource_put_bytes);
    method!("resourceDelete", resource_delete);
    method!("formXml", form_xml);
    method!("isLastRunBeforeLastSchedule", is_last_run_before_last_schedule);
    method!("prettyToCamelCase", pretty_to_camel_case);
    let define: Function = eval_js(ctx, TOOLS_JS)?;
    let ec: Object = caught(ctx, "ec", ctx.globals().get("ec"))?;
    caught(
        ctx,
        "tools",
        define.call::<_, ()>((
            ec,
            collection_utilities.clone(),
            string_utilities.clone(),
            web_utilities.clone(),
            host,
        )),
    )
}
