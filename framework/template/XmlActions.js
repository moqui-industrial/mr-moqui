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

// One function per concrete xml-actions element. The Rust walker calls
// macros[name](node, children). Node is {name, attrs, text, children}.
// Each child already has its rendered source. Statements use bare names.
(function () {
    "use strict";

    function attr(node, name) {
        var attrs = node.attrs || {};
        return attrs[name] == null ? "" : String(attrs[name]);
    }

    function has(node, name) {
        return !!(node.attrs && Object.prototype.hasOwnProperty.call(node.attrs, name));
    }

    function jsString(text) {
        return JSON.stringify(text == null ? "" : String(text));
    }

    function expandExpr(text) {
        return "ec.resource.expand(" + jsString(text) + ")";
    }

    function literalOrExpand(text) {
        var value = text == null ? "" : String(text);
        if (value.indexOf("$") >= 0 && value.charAt(value.length - 1) !== "$") return expandExpr(value);
        return jsString(value);
    }

    function jsBool(text) {
        if (text === "true" || text === "false") return text;
        return text;
    }

    function child(node, name) {
        var kids = node.children || [];
        for (var i = 0; i < kids.length; i++) {
            if (kids[i].name === name) return kids[i];
        }
        return null;
    }

    function childrenNamed(node, name) {
        var found = [];
        var kids = node.children || [];
        for (var i = 0; i < kids.length; i++) {
            if (kids[i].name === name) found.push(kids[i]);
        }
        return found;
    }

    function sourcesOf(list) {
        var out = "";
        var items = list || [];
        for (var i = 0; i < items.length; i++) out += items[i].source || "";
        return out;
    }

    function valueExpr(node) {
        var base;
        if (has(node, "from")) base = attr(node, "from");
        else if (has(node, "value")) base = expandExpr(attr(node, "value"));
        else base = "null";
        if (has(node, "default-value")) {
            var fallback = expandExpr(attr(node, "default-value"));
            base = "((" + base + " == null || " + base + " === \"\") ? " + fallback + " : (" + base + "))";
        }
        var type = attr(node, "type");
        if (type === "NewList") return "[]";
        if (type === "NewMap") return "{}";
        if (type) return "basicConvert(" + base + ", " + jsString(type) + ")";
        return base;
    }

    function fieldExpr(node) {
        if (has(node, "from")) return attr(node, "from");
        if (has(node, "value")) return expandExpr(attr(node, "value"));
        return attr(node, "field-name");
    }

    function econditionFactory(node) {
        var fromExpr = has(node, "from") ? attr(node, "from") : fieldExpr(node);
        return "ec.entity.conditionFactory.makeActionConditionDirect(" + jsString(attr(node, "field-name")) + ", " +
            jsString(attr(node, "operator") || "equals") + ", " + fromExpr + ", " +
            (has(node, "value") ? jsString(attr(node, "value")) : "null") + ", " +
            (has(node, "to-field-name") ? jsString(attr(node, "to-field-name")) : "null") + ", " +
            (attr(node, "ignore-case") === "true") + ", " + (attr(node, "ignore-if-empty") === "true") + ", " +
            (attr(node, "or-null") === "true") + ", " + jsString(attr(node, "ignore") || "false") + ")";
    }

    function econditionSuffix(node) {
        var operator = attr(node, "operator") || "equals";
        var simple = operator === "equals" && attr(node, "ignore-case") !== "true" &&
            attr(node, "ignore-if-empty") !== "true" && attr(node, "or-null") !== "true" &&
            !has(node, "to-field-name");
        if (simple) return ".condition(" + jsString(attr(node, "field-name")) + ", " + fieldExpr(node) + ")";
        return ".condition(" + econditionFactory(node) + ")";
    }

    function dateFilterExpr(node) {
        var valid = has(node, "valid-date") ? attr(node, "valid-date") : "null";
        return "ec.entity.conditionFactory.makeConditionDate(" + jsString(attr(node, "from-field-name") || "fromDate") +
            ", " + jsString(attr(node, "thru-field-name") || "thruDate") + ", " + valid + ", " +
            (attr(node, "ignore-if-empty") === "true") + ", " + jsString(attr(node, "ignore") || "false") + ")";
    }

    function conditionObjectExpr(node) {
        var parts = [];
        var kids = node.children || [];
        for (var i = 0; i < kids.length; i++) {
            if (kids[i].name === "econdition") parts.push(econditionFactory(kids[i]));
            else if (kids[i].name === "date-filter") parts.push(dateFilterExpr(kids[i]));
            else if (kids[i].name === "econdition-object") parts.push(attr(kids[i], "field") || "null");
            else parts.push(kids[i].source || "null");
        }
        return "ec.entity.conditionFactory.makeCondition([" + parts.join(", ") + "], " +
            jsString(attr(node, "combine") || "and") + ")";
    }

    function findPrefix(node) {
        var chain = "ec.entity.find(" + jsString(attr(node, "entity-name")) + ")";
        if (has(node, "cache")) chain += ".useCache(" + jsBool(attr(node, "cache")) + ")";
        if (has(node, "for-update")) chain += ".forUpdate(" + jsBool(attr(node, "for-update")) + ")";
        if (has(node, "distinct")) chain += ".distinct(" + jsBool(attr(node, "distinct")) + ")";
        if (has(node, "use-clone")) chain += ".useClone(" + jsBool(attr(node, "use-clone")) + ")";
        if (has(node, "offset")) chain += ".offset(" + attr(node, "offset") + ")";
        if (has(node, "limit")) chain += ".limit(" + attr(node, "limit") + ")";
        var kids = node.children || [];
        for (var i = 0; i < kids.length; i++) {
            if (kids[i].name === "select-field") chain += ".selectField(" + jsString(attr(kids[i], "field-name")) + ")";
        }
        for (var o = 0; o < kids.length; o++) {
            if (kids[o].name === "order-by") chain += ".orderBy(" + jsString(attr(kids[o], "field-name")) + ")";
        }
        var cached = attr(node, "cache") === "true";
        for (var c = 0; c < kids.length; c++) {
            var name = kids[c].name;
            if (name === "date-filter" && cached) continue;
            if (name === "econdition") chain += kids[c].source || econditionSuffix(kids[c]);
            else if (name === "econditions") chain += kids[c].source || (".condition(" + conditionObjectExpr(kids[c]) + ")");
            else if (name === "date-filter") chain += kids[c].source || (".condition(" + dateFilterExpr(kids[c]) + ")");
            else if (name === "econdition-object" && attr(kids[c], "field")) chain += ".condition(" + attr(kids[c], "field") + ")";
            else if (name === "having-econditions") chain += kids[c].source || "";
        }
        return chain;
    }

    function searchFormLines(listName, node) {
        var input = has(node, "input-fields-map") ? attr(node, "input-fields-map") : "context";
        var defs = "null";
        var defaults = child(node, "default-parameters");
        if (defaults) {
            var parts = [];
            var attrs = defaults.attrs || {};
            for (var key in attrs) {
                if (Object.prototype.hasOwnProperty.call(attrs, key)) parts.push(jsString(key) + ": " + expandExpr(String(attrs[key])));
            }
            defs = "{" + parts.join(", ") + "}";
        }
        var paginate = has(node, "paginate") ? jsBool(attr(node, "paginate")) : "true";
        var lines = "    if (true) {\n";
        if (has(node, "require-parameters")) {
            lines += "        " + listName + "_xafind.requireSearchFormParameters(" + expandExpr(attr(node, "require-parameters")) + " === \"true\");\n";
        }
        lines += "        " + listName + "_xafind.searchFormMap(" + input + ", " + defs + ", " +
            jsString(attr(node, "skip-fields")) + ", " + jsString(attr(node, "default-order-by")) + ", " + paginate + ");\n";
        lines += "    }\n";
        return lines;
    }

    function conditionExpr(node) {
        var parts = [];
        if (has(node, "condition") && attr(node, "condition") !== "") parts.push(attr(node, "condition"));
        var cond = child(node, "condition");
        if (cond) {
            var inner = sourcesOf(cond.children);
            if (inner) parts.push(inner);
        }
        if (!parts.length) return "true";
        return parts.join(" && ");
    }

    function blockBody(node) {
        var out = "";
        var kids = node.children || [];
        for (var i = 0; i < kids.length; i++) {
            var name = kids[i].name;
            if (name === "condition" || name === "then" || name === "else" || name === "else-if") continue;
            out += kids[i].source || "";
        }
        var thens = childrenNamed(node, "then");
        for (var t = 0; t < thens.length; t++) out += sourcesOf(thens[t].children);
        return out;
    }

    function compareCall(node) {
        var op = has(node, "operator") ? jsString(attr(node, "operator")) : "\"equals\"";
        var value = has(node, "value") ? literalOrExpand(attr(node, "value")) : "null";
        var toField = has(node, "to-field") ? attr(node, "to-field") : "null";
        var format = has(node, "format") ? jsString(attr(node, "format")) : "null";
        var type = has(node, "type") ? jsString(attr(node, "type")) : "\"Object\"";
        return "compare(" + attr(node, "field") + ", " + op + ", " + value + ", " + toField + ", " + format + ", " + type + ")";
    }

    function messageCall(error, isPublic, type, text) {
        var expanded = expandExpr(text);
        if (error) return "    ec.message.addError(" + expanded + ");\n";
        if (isPublic) return "    ec.message.addPublic(" + expanded + ", " + jsString(type || "info") + ");\n";
        return "    ec.message.addMessage(" + expanded + ", " + jsString(type || "info") + ");\n";
    }

    return {
        "actions": function (_node, children) {
            return "(function() {\n" + children + "    return;\n})()\n";
        },
        "condition": function () { return ""; },
        "then": function () { return ""; },
        "else": function () { return ""; },
        "else-if": function () { return ""; },
        "service-call": function (node) {
            var name = attr(node, "name");
            var isAsync = has(node, "async") && attr(node, "async") !== "false";
            var chain = "ec.service." + (isAsync ? "async()" : "sync()") + ".name(" + jsString(name) + ")";
            // multi="parameter" would read ec.web. That host is not added.
            if (!isAsync && attr(node, "disable-authz") === "true") chain += ".disableAuthz()";
            if (!isAsync && attr(node, "multi") === "true") chain += ".multi(true)";
            if (!isAsync && has(node, "transaction")) {
                var tx = attr(node, "transaction");
                if (tx === "ignore") chain += ".ignoreTransaction(true)";
                else if (tx === "force-new" || tx === "force-cache") chain += ".requireNewTransaction(true)";
                if (has(node, "transaction-timeout")) chain += ".transactionTimeout(" + attr(node, "transaction-timeout") + ")";
                chain += (tx === "cache" || tx === "force-cache") ? ".useTransactionCache(true)" : ".useTransactionCache(false)";
            }
            if (attr(node, "in-map") === "true") chain += ".parameters(context)";
            else if (has(node, "in-map") && attr(node, "in-map") !== "false" && attr(node, "in-map") !== "") {
                chain += ".parameters(" + attr(node, "in-map") + ")";
            }
            var maps = childrenNamed(node, "field-map");
            for (var i = 0; i < maps.length; i++) {
                chain += ".parameter(" + jsString(attr(maps[i], "field-name")) + ", " + fieldExpr(maps[i]) + ")";
            }
            chain += ".call()";
            var lines = "    if (true) {\n";
            var outMap = attr(node, "out-map");
            if (outMap && !isAsync) {
                lines += "        var call_service_result = " + chain + ";\n";
                var add = !has(node, "out-map-add-to-existing") || attr(node, "out-map-add-to-existing") === "true";
                if (add) {
                    lines += "        if (" + outMap + " != null) { if (call_service_result) Object.assign(" + outMap +
                        ", call_service_result); } else { " + outMap + " = call_service_result; }\n";
                } else lines += "        " + outMap + " = call_service_result;\n";
            } else lines += "        " + chain + ";\n";
            if (attr(node, "ignore-error") === "true") {
                lines += "        if (ec.message.hasError()) {\n";
                lines += "            ec.logger.warn(" + jsString("Ignoring error running service " + name + ": ") +
                    " + ec.message.getErrorsString());\n";
                lines += "            ec.message.clearErrors();\n";
                lines += "        }\n";
            } else lines += "        if (ec.message.hasError()) return;\n";
            lines += "    }\n";
            return lines;
        },
        "script": function (node) {
            var out = "";
            if (has(node, "location") && attr(node, "location") !== "") {
                out += "    ec.resource.script(" + jsString(attr(node, "location")) + ");\n";
            }
            out += "// begin inline script\n" + (node.text || "") + "\n// end inline script\n";
            return out;
        },
        "set": function (node) {
            var field = attr(node, "field");
            var expr = valueExpr(node);
            if (attr(node, "set-if-empty") === "false") {
                return "    var _temp_internal = " + expr + ";\n    if (_temp_internal != null && _temp_internal !== \"\") " +
                    field + " = _temp_internal;\n";
            }
            return "    " + field + " = " + expr + ";\n";
        },
        "order-map-list": function (node) {
            var fields = childrenNamed(node, "order-by").map(function (ob) { return jsString(attr(ob, "field-name")); });
            return "    orderMapList(" + attr(node, "list") + ", [" + fields.join(", ") + "]);\n";
        },
        "filter-map-list": function (node) {
            var list = attr(node, "list");
            var lines = "    if (" + list + " != null) {\n";
            if (has(node, "to-list")) {
                lines += "        var _listToFilter = " + attr(node, "to-list") + " = " + list + ".slice();\n";
            } else lines += "        var _listToFilter = " + list + ";\n";
            var maps = childrenNamed(node, "field-map");
            if (maps.length) {
                var parts = [];
                for (var i = 0; i < maps.length; i++) {
                    parts.push(jsString(attr(maps[i], "field-name")) + ": " + fieldExpr(maps[i]));
                }
                lines += "        filterMapList(_listToFilter, {" + parts.join(", ") + "});\n";
            }
            var dates = childrenNamed(node, "date-filter");
            for (var d = 0; d < dates.length; d++) {
                var valid = has(dates[d], "valid-date") ? attr(dates[d], "valid-date") : "null";
                lines += "        filterMapListByDate(_listToFilter, " +
                    jsString(attr(dates[d], "from-field-name") || "fromDate") + ", " +
                    jsString(attr(dates[d], "thru-field-name") || "thruDate") + ", " + valid + ", " +
                    (attr(dates[d], "ignore-if-empty") === "true") + ");\n";
            }
            lines += "    }\n";
            return lines;
        },
        "date-filter": function (node) {
            return ".condition(" + dateFilterExpr(node) + ")";
        },
        "entity-data": function () {
            return "throw new Error(\"entity-data is not implemented\");\n";
        },
        "entity-find-one": function (node) {
            var chain = "ec.entity.find(" + jsString(attr(node, "entity-name")) + ")";
            if (has(node, "cache")) chain += ".useCache(" + jsBool(attr(node, "cache")) + ")";
            if (has(node, "for-update")) chain += ".forUpdate(" + jsBool(attr(node, "for-update")) + ")";
            if (has(node, "use-clone")) chain += ".useClone(" + jsBool(attr(node, "use-clone")) + ")";
            var maps = childrenNamed(node, "field-map");
            if (has(node, "auto-field-map")) {
                var auto = attr(node, "auto-field-map");
                if (auto === "true") chain += ".condition(context)";
                else if (auto !== "false" && auto !== "") chain += ".condition(" + auto + ")";
            } else if (!maps.length) chain += ".condition(context)";
            for (var i = 0; i < maps.length; i++) {
                chain += ".condition(" + jsString(attr(maps[i], "field-name")) + ", " + fieldExpr(maps[i]) + ")";
            }
            var selected = childrenNamed(node, "select-field");
            for (var s = 0; s < selected.length; s++) chain += ".selectField(" + jsString(attr(selected[s], "field-name")) + ")";
            return "    var " + attr(node, "value-field") + " = " + chain + ".one();\n";
        },
        "entity-find": function (node) {
            var list = attr(node, "list");
            var lines = "    var " + list + "_xafind = " + findPrefix(node) + ";\n";
            var sfi = child(node, "search-form-inputs");
            if (sfi) lines += searchFormLines(list, sfi);
            var cached = attr(node, "cache") === "true";
            var limitRange = child(node, "limit-range");
            var limitView = child(node, "limit-view");
            var useIter = child(node, "use-iterator");
            if (limitRange && !cached) {
                lines += "    var " + list + " = " + list + "_xafind.iterator().getPartialList(" +
                    attr(limitRange, "start") + ", " + attr(limitRange, "size") + ", true);\n";
            } else if (limitView && !cached) {
                var index = attr(limitView, "view-index");
                var size = attr(limitView, "view-size");
                lines += "    var " + list + " = " + list + "_xafind.iterator().getPartialList((" + index + " - 1) * " +
                    size + ", " + size + ", true);\n";
            } else if (useIter && !cached) {
                lines += "    var " + list + " = " + list + "_xafind.iterator();\n";
            } else {
                lines += "    var " + list + " = " + list + "_xafind.list();\n";
                if (cached) {
                    var dates = childrenNamed(node, "date-filter");
                    for (var i = 0; i < dates.length; i++) {
                        var valid = has(dates[i], "valid-date") ? attr(dates[i], "valid-date") : "null";
                        lines += "    " + list + " = " + list + ".filterByDate(" +
                            jsString(attr(dates[i], "from-field-name") || "fromDate") + ", " +
                            jsString(attr(dates[i], "thru-field-name") || "thruDate") + ", " + valid + ", " +
                            (attr(dates[i], "ignore-if-empty") === "true") + ");\n";
                    }
                }
            }
            return lines;
        },
        "search-form-inputs": function () { return ""; },
        "default-parameters": function () { return ""; },
        "econditions": function (node) {
            return ".condition(" + conditionObjectExpr(node) + ")";
        },
        "having-econditions": function (node) {
            var out = "";
            var kids = node.children || [];
            for (var i = 0; i < kids.length; i++) {
                if (kids[i].name === "econdition") out += ".havingCondition(" + econditionFactory(kids[i]) + ")";
                else if (kids[i].name === "date-filter") out += ".havingCondition(" + dateFilterExpr(kids[i]) + ")";
                else if (kids[i].name === "econditions") out += ".havingCondition(" + conditionObjectExpr(kids[i]) + ")";
                else if (kids[i].name === "econdition-object" && attr(kids[i], "field")) {
                    out += ".havingCondition(" + attr(kids[i], "field") + ")";
                }
            }
            return out;
        },
        "econdition": function (node) { return econditionSuffix(node); },
        "econdition-object": function (node) {
            var field = attr(node, "field");
            return field ? ".condition(" + field + ")" : "";
        },
        "select-field": function () { return ""; },
        "order-by": function () { return ""; },
        "limit-range": function () { return ""; },
        "limit-view": function () { return ""; },
        "use-iterator": function () { return ""; },
        "field-map": function () { return ""; },
        "entity-find-count": function (node) {
            var chain = findPrefix(node);
            var sfi = child(node, "search-form-inputs");
            if (!sfi) return "    " + attr(node, "count-field") + " = " + chain + ".count();\n";
            return "    var _count_xafind = " + chain + ";\n" + searchFormLines("_count", sfi) +
                "    " + attr(node, "count-field") + " = _count_xafind.count();\n";
        },
        "entity-find-related-one": function (node) {
            var value = attr(node, "value-field");
            var cache = has(node, "cache") ? attr(node, "cache") : "null";
            var forUpdate = has(node, "for-update") ? attr(node, "for-update") : "null";
            return "    var " + attr(node, "to-value-field") + " = (" + value + " == null) ? null : " + value +
                ".findRelatedOne(" + jsString(attr(node, "relationship-name")) + ", " + cache + ", " + forUpdate + ");\n";
        },
        "entity-find-related": function (node) {
            var value = attr(node, "value-field");
            var map = has(node, "map") ? attr(node, "map") : "null";
            var order = has(node, "order-by-list") ? attr(node, "order-by-list") : "null";
            var cache = has(node, "cache") ? attr(node, "cache") : "null";
            var forUpdate = has(node, "for-update") ? attr(node, "for-update") : "null";
            return "    var " + attr(node, "list") + " = (" + value + " == null) ? null : " + value + ".findRelated(" +
                jsString(attr(node, "relationship-name")) + ", " + map + ", " + order + ", " + cache + ", " + forUpdate + ");\n";
        },
        "entity-make-value": function (node) {
            var field = attr(node, "value-field");
            var lines = "    var " + field + " = ec.entity.makeValue(" + jsString(attr(node, "entity-name")) + ");\n";
            if (has(node, "map") && attr(node, "map") !== "") {
                lines += "    " + field + ".setFields(" + attr(node, "map") + ", true, null, null);\n";
            }
            return lines;
        },
        "entity-create": function (node) {
            var call = attr(node, "or-update") === "true" ? ".createOrUpdate()" : ".create()";
            return "    " + attr(node, "value-field") + call + ";\n";
        },
        "entity-update": function (node) {
            return "    " + attr(node, "value-field") + ".update();\n";
        },
        "entity-delete": function (node) {
            return "    " + attr(node, "value-field") + ".delete();\n";
        },
        "entity-delete-related": function (node) {
            return "    " + attr(node, "value-field") + ".deleteRelated(" + jsString(attr(node, "relationship-name")) + ");\n";
        },
        "entity-delete-by-condition": function (node) {
            return "    " + findPrefix(node) + ".deleteAll();\n";
        },
        "entity-set": function (node) {
            var map = has(node, "map") ? attr(node, "map") : "context";
            var prefix = has(node, "prefix") && attr(node, "prefix") !== "" ? attr(node, "prefix") : "null";
            var include = attr(node, "include");
            var flag = include === "pk" ? "true" : (include === "nonpk" ? "false" : "null");
            return "    " + attr(node, "value-field") + ".setFields(" + map + ", " +
                (attr(node, "set-if-empty") === "true") + ", " + prefix + ", " + flag + ");\n";
        },
        "entity-sequenced-id-primary": function (node) {
            return "    " + attr(node, "value-field") + ".setSequencedIdPrimary();\n";
        },
        "entity-sequenced-id-secondary": function (node) {
            return "    " + attr(node, "value-field") + ".setSequencedIdSecondary();\n";
        },
        "break": function () { return "    break;\n"; },
        "continue": function () { return "    continue;\n"; },
        "iterate": function (node) {
            var list = attr(node, "list");
            var entry = attr(node, "entry");
            var key = attr(node, "key");
            var body = sourcesOf(node.children);
            if (key) {
                return "    if (" + list + " != null) {\n" +
                    "        var " + entry + "_index = 0;\n" +
                    "        var _" + entry + "Entries = (typeof " + list + ".entries === \"function\") ? " + list +
                    ".entries() : Object.entries(" + list + ");\n" +
                    "        while (" + entry + "_index < _" + entry + "Entries.length) {\n" +
                    "            var " + entry + "Entry = _" + entry + "Entries[" + entry + "_index];\n" +
                    "            " + key + " = " + entry + "Entry[0];\n" +
                    "            var " + entry + " = " + entry + "Entry[1];\n" +
                    "            " + entry + "_has_next = (" + entry + "_index + 1) < _" + entry + "Entries.length;\n" +
                    body +
                    "            " + entry + "_index = " + entry + "_index + 1;\n" +
                    "        }\n" +
                    "    }\n";
            }
            return "    if (" + list + " != null) {\n" +
                "        var " + entry + "_index = 0;\n" +
                "        var " + entry + "_len = (typeof " + list + ".length === \"number\") ? " + list + ".length : 0;\n" +
                "        while (" + entry + "_index < " + entry + "_len) {\n" +
                "            var " + entry + " = (typeof " + list + ".get === \"function\") ? " + list + ".get(" +
                entry + "_index) : " + list + "[" + entry + "_index];\n" +
                "            " + entry + "_has_next = (" + entry + "_index + 1) < " + entry + "_len;\n" +
                body +
                "            " + entry + "_index = " + entry + "_index + 1;\n" +
                "        }\n" +
                "    }\n";
        },
        "message": function (node) {
            return messageCall(attr(node, "error") === "true", attr(node, "public") === "true", attr(node, "type"), node.text || "");
        },
        "check-errors": function () {
            return "    if (ec.message.hasError()) return;\n";
        },
        "return": function (node) {
            var out = "";
            if (has(node, "message") && attr(node, "message") !== "") {
                out += messageCall(attr(node, "error") === "true", attr(node, "public") === "true", attr(node, "type"), attr(node, "message"));
            }
            out += "    return;\n";
            return out;
        },
        "assert": function (node) {
            var title = attr(node, "title");
            var prefix = title ? "[" + title + "] " : "";
            var out = "";
            var kids = node.children || [];
            for (var i = 0; i < kids.length; i++) {
                var src = kids[i].source || "true";
                out += "    if (!(" + src + ")) ec.message.addError(" + expandExpr(prefix + "Assert failed: " + src) + ");\n";
            }
            return out;
        },
        "if": function (node) {
            var src = "    if (" + conditionExpr(node) + ") {\n" + blockBody(node);
            var elseIfs = childrenNamed(node, "else-if");
            for (var i = 0; i < elseIfs.length; i++) {
                src += "    } else if (" + conditionExpr(elseIfs[i]) + ") {\n" + blockBody(elseIfs[i]);
            }
            var elses = childrenNamed(node, "else");
            if (elses.length) src += "    } else {\n" + sourcesOf(elses[0].children);
            src += "    }\n";
            return src;
        },
        "while": function (node) {
            return "    while (" + conditionExpr(node) + ") {\n" + blockBody(node) + "    }\n";
        },
        "or": function (node) {
            var parts = [];
            var kids = node.children || [];
            for (var i = 0; i < kids.length; i++) parts.push(kids[i].source || "true");
            return "(" + (parts.length ? parts.join(" || ") : "true") + ")";
        },
        "and": function (node) {
            var parts = [];
            var kids = node.children || [];
            for (var i = 0; i < kids.length; i++) parts.push(kids[i].source || "true");
            return "(" + (parts.length ? parts.join(" && ") : "true") + ")";
        },
        "not": function (node) {
            var kids = node.children || [];
            return "!(" + (kids.length ? (kids[0].source || "true") : "true") + ")";
        },
        "compare": function (node) {
            var call = compareCall(node);
            var kids = node.children || [];
            var body = "";
            var hasElse = false;
            var elseSrc = "";
            for (var i = 0; i < kids.length; i++) {
                if (kids[i].name === "else") {
                    hasElse = true;
                    elseSrc = sourcesOf(kids[i].children);
                } else body += kids[i].source || "";
            }
            if (!body && !hasElse) return call;
            var src = "    if (" + call + ") {\n" + body;
            if (hasElse) src += "    } else {\n" + elseSrc;
            src += "    }\n";
            return src;
        },
        "expression": function (node) {
            var text = node.text || "";
            var kids = node.children || [];
            if (!kids.length) return text;
            return "    if (" + (text || "true") + ") {\n" + sourcesOf(kids) + "    }\n";
        },
        "log": function (node) {
            var level = has(node, "level") && attr(node, "level") !== "" ? attr(node, "level") : "info";
            return "    ec.logger.log(" + jsString(level) + ", " + expandExpr(attr(node, "message")) + ", null);\n";
        }
    };
})()
