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

// The tui render mode: one JSON array per section. A parent puts a nested section in a value
// (`"c":` and then the child array). Exit widgets are real nodes. Anything else is a structural
// node or a placeholder, still valid JSON. A form macro writes its fields; widget macros do not
// also write them.
(function (T, S) {
    "use strict";

    const { attr, has, jsString, child, childrenNamed, stmt } = T;

    const prelude = `
let $comma = false;
const $stack = [];
const $sep = () => { if ($comma) sri.write(","); $comma = true; };
const $clean = (value) => {
    const text = String(value == null ? "" : value);
    let out = "";
    for (const ch of text) {
        const code = ch.codePointAt(0);
        if (code === 9 || code === 10 || (code >= 32 && code !== 127 && (code < 128 || code > 159))) out += ch;
    }
    return out;
};
const $scrub = (value) => {
    if (typeof value === "string") return JSON.stringify($clean(value));
    if (value === null || value === undefined) return "null";
    if (typeof value === "number" || typeof value === "boolean") return JSON.stringify(value);
    if (Array.isArray(value)) return "[" + value.map($scrub).join(",") + "]";
    if (typeof value === "object") {
        let out = "{";
        let first = true;
        for (const key of Object.keys(value)) {
            if (!first) out += ",";
            first = false;
            out += JSON.stringify(key) + ":" + $scrub(value[key]);
        }
        return out + "}";
    }
    return JSON.stringify($clean(String(value)));
};
const $emit = (value) => { $sep(); sri.write($scrub(value)); };
const $openRender = (fields) => { $sep(); sri.write($scrub(fields).slice(0, -1) + ',"c":'); };
const $closeRender = () => { sri.write("}"); };
const $openChildren = (fields) => {
    $sep();
    sri.write($scrub(fields).slice(0, -1) + ',"c":[');
    $stack.push($comma);
    $comma = false;
};
const $closeChildren = () => {
    sri.write("]");
    $comma = $stack.pop();
    sri.write("}");
};
const $whole = (value) => {
    const n = Number(value);
    return Number.isFinite(n) && n >= 0 ? Math.trunc(n) : 0;
};
const $secret = new Set(["password", "oldpassword", "newpassword", "newpasswordverify", "currentpassword", "resetpassword", "authpassword", "api_key", "login_key", "passwordsalt", "passwordhashtype", "passwordbase64", "passwordverify"]);
sri.write("[");
`;
    const epilogue = `
sri.write("]");
`;

    const SECRET = new Set(["password", "oldpassword", "newpassword", "newpasswordverify", "currentpassword", "resetpassword", "authpassword", "api_key", "login_key", "passwordsalt", "passwordhashtype", "passwordbase64", "passwordverify"]);
    const typeNames = (type) => (type || "").split(",").map((name) => name.trim()).filter((name) => name);
    const exprOf = (text) => (text && String(text).includes("${") ? S.expandExpr(text) : jsString(text || ""));
    const guarded = (node, code) => {
        const condition = attr(node, "condition");
        if (!condition || !code) return code;
        return `if (${condition}) {\n${code}}\n`;
    };

    const elementPath = (node) => {
        const parts = [];
        let current = node;
        while (current && current.name !== "screen") {
            const siblings = current.parent ? current.parent.children.filter((kid) => kid.name === current.name) : [current];
            const label = attr(current, "id") || attr(current, "name") || current.name;
            const index = siblings.indexOf(current);
            const numbered = siblings.length > 1 && !attr(current, "id") && !attr(current, "name");
            parts.push(numbered ? label + "[" + index + "]" : label);
            current = current.parent;
        }
        return parts.reverse().join("/");
    };
    const idOf = (node) => attr(node, "id") || attr(node, "name") || elementPath(node);
    const formNodeOf = (node) => {
        let current = node;
        while (current) {
            if (current.name === "form-single" || current.name === "form-list") return current;
            current = current.parent;
        }
        return null;
    };
    const fieldNodeOf = (node) => {
        let current = node;
        while (current) {
            if (current.name === "field") return current;
            current = current.parent;
        }
        return null;
    };
    const formNameOf = (node) => attr(node, "name") || (node.name === "form-list" ? "list" : "form");
    const fieldIdOf = (field) => {
        if (attr(field, "id")) return attr(field, "id");
        const form = field.parent;
        const formName = form ? formNameOf(form) : "form";
        return formName + "." + (attr(field, "name") || "field");
    };
    const inHeader = (node) => {
        let current = node;
        while (current) {
            if (current.name === "header-field") return true;
            if (current.name === "form-single" || current.name === "form-list") return false;
            current = current.parent;
        }
        return false;
    };
    const maxlengthOf = (sub, widget) => {
        const text = attr(widget, "maxlength") || attr(sub, "maxlength") || "";
        return /^\d+$/.test(text) ? text : "0";
    };
    const fallbackKey = (widget) => {
        if (!has(widget, "no-current-selected-key")) return '""';
        if (widget.name === "check") return S.expandNoL10nExpr(attr(widget, "no-current-selected-key"));
        return jsString(attr(widget, "no-current-selected-key"));
    };
    const parameterNames = (dynamic) => {
        const names = [];
        const add = (name) => { if (name && !names.includes(name)) names.push(name); };
        childrenNamed(dynamic, "depends-on").forEach((dep) => add(attr(dep, "field")));
        const map = attr(dynamic, "parameter-map");
        if (map) {
            const re = /([A-Za-z_][A-Za-z0-9_]*)\s*:/g;
            let found = re.exec(map);
            while (found) {
                if (found[1] !== "context") add(found[1]);
                found = re.exec(map);
            }
        }
        if (attr(dynamic, "server-search") === "true") add("term");
        return names;
    };

    const screenUrlExpr = `(() => { const $info = sri.getScreenUrlInfo(); const $names = $info && $info.fullPathNameList; if ($names && $names.length) return "/" + $names.join("/"); const $screen = sri.getActiveScreenDef(); return ($screen && $screen.location) || ""; })()`;

    const linkIdExpr = (node, inList) => {
        if (attr(node, "id")) return jsString(attr(node, "id"));
        const field = fieldNodeOf(node);
        if (field) {
            const base = fieldIdOf(field);
            if (inList) return jsString(base + ".") + " + $rowKey";
            return jsString(base);
        }
        return jsString(idOf(node));
    };
    const linkExpr = (node, inList) => {
        const kind = attr(node, "link-type") || "";
        const confirm = attr(node, "confirmation") || "";
        return `(() => {
            let $text = ${S.linkTextExpr(node)};
            if ($text == null || $text === "null") $text = "";
            const $made = sri.makeUrlByType(${jsString(attr(node, "url") || "")}, ${jsString(attr(node, "url-type") || "transition")}, ${S.nodeLit(node, true)}, ${jsString(attr(node, "expand-transition-url") || "true")});
            const $link = {t:"link",id:${linkIdExpr(node, inList)},text:String($text),url:$made ? ($made.url || "") : ""};
            ${kind ? `$link.kind = ${jsString(kind)};` : ""}
            ${kind === "hidden-form-link" ? `$link.method = "post";` : ""}
            ${confirm ? `$link.confirm = String(${exprOf(confirm)} || "");` : ""}
            return $link;
        })()`;
    };
    const linkStatement = (node, inList) => guarded(node, `$emit(${linkExpr(node, inList)});\n`);

    const valueCall = (widget) => `sri.getFieldValueString(${S.nodeLit(widget, false)}) || ${fallbackKey(widget)}`;
    const inlineOptions = (widget, kind) => `(() => {
        const $value = String(${valueCall(widget)} || "");
        const $options = [];
        const $map = sri.getFieldOptions(${S.nodeLit(widget, true)});
        if ($map && $map.forEach) $map.forEach(($label, $key) => $options.push({value:String($key),label:String($label == null ? "" : $label)}));
        if ($value && !$options.some(($opt) => $opt.value === $value)) $options.push({value:$value,label:$value});
        return {kind:${jsString(kind)},value:$value,options:$options};
    })()`;
    const dynamicOptions = (widget) => {
        const dynamic = child(widget, "dynamic-options");
        const depends = childrenNamed(dynamic, "depends-on").map((dep) => attr(dep, "field")).filter(Boolean);
        const labelField = attr(dynamic, "label-field") || "label";
        return `(() => {
            const $value = String(${valueCall(widget)} || "");
            const $made = sri.makeUrlByType(${jsString(attr(dynamic, "transition") || "")}, "transition", ${S.nodeLit(dynamic, true)}, "true");
            const $optionsUrl = $made ? ($made.url || "") : "";
            const $depends = ${JSON.stringify(depends)};
            const $params = ${JSON.stringify(parameterNames(dynamic))};
            let $label = $value;
            if ($value) {
                const $found = sri.getFieldTransitionValue(${jsString(attr(dynamic, "transition") || "")}, ${S.nodeLit(dynamic, true)}, $value, ${jsString(labelField)}, false);
                if ($found) $label = String($found);
            }
            const $w = {kind:"dropDown",value:$value,options:$value ? [{value:$value,label:$label}] : []};
            if ($optionsUrl) $w.optionsUrl = $optionsUrl;
            if ($depends.length) $w.dependsOn = $depends;
            if ($params.length) $w.optionsParameters = $params;
            return $w;
        })()`;
    };
    const widgetExpr = (widget) => {
        const field = fieldNodeOf(widget);
        const secret = field && SECRET.has((attr(field, "name") || "").toLowerCase());
        if (widget.name === "password") return '{kind:"password"}';
        if (secret) {
            if (widget.name === "display" || widget.name === "display-entity") return '{kind:"display",text:""}';
            const kind = widget.name === "text-find" ? "textFind" : widget.name === "hidden" ? "hidden" : "textLine";
            return `{kind:${jsString(kind)},value:""}`;
        }
        if (widget.name === "text-line" || widget.name === "text-area") {
            return `{kind:"textLine",value:String(${valueCall(widget)} || "")}`;
        }
        if (widget.name === "text-find") return `{kind:"textFind",value:String(${valueCall(widget)} || "")}`;
        if (widget.name === "drop-down") return child(widget, "dynamic-options") ? dynamicOptions(widget) : inlineOptions(widget, "dropDown");
        if (widget.name === "radio") return inlineOptions(widget, "radio");
        if (widget.name === "check") return `{kind:"check",value:String(${valueCall(widget)} || "")}`;
        if (widget.name === "date-time") {
            return `{kind:"dateTime",value:String(sri.getFieldValueString(${S.fieldLit(widget)}, ${jsString(attr(widget, "default-value"))}, ${jsString(S.dateTimeFormat(widget))}, ${jsString(attr(widget, "type") || "date-time")}) || ""),format:${jsString(S.dateTimeFormat(widget))}}`;
        }
        if (widget.name === "display") return `{kind:"display",text:String(${S.displayValueExpr(widget)} || "")}`;
        if (widget.name === "display-entity") return `{kind:"display",text:String(sri.getFieldEntityValue(${S.nodeLit(widget, false)}) || "")}`;
        if (widget.name === "hidden") return `{kind:"hidden",value:String(${valueCall(widget)} || "")}`;
        if (widget.name === "submit" || widget.name === "reset") {
            return `{kind:${jsString(widget.name)},text:String(${S.localizedTitle(widget.parent)} || "")}`;
        }
        return "";
    };
    const fieldObject = (field, sub, widget) => {
        const w = widgetExpr(widget);
        if (!w) return "";
        const required = attr(field, "required") === "true" || attr(sub, "required") === "true";
        return `(() => {
            let $required = ${required ? "true" : "false"};
            const $max = ${maxlengthOf(sub, widget)};
            const $hint = sri.getFieldValidation(${S.nodeLit(sub, false)});
            if ($hint && String($hint.classes || "").split(/\\s+/).indexOf("required") >= 0) $required = true;
            const $rules = sri.getFieldValidationJsRules(${S.nodeLit(sub, false)}) || [];
            for (let $ri = 0; $ri < $rules.length; $ri++) if ($rules[$ri] && $rules[$ri].expr === "!!value") $required = true;
            const $field = {t:"field",id:${jsString(fieldIdOf(field))},name:${jsString(attr(field, "name") || "field")},title:String(${S.localizedTitle(sub)} || ""),w:${w}};
            if ($required) $field.required = true;
            if ($max > 0) $field.maxlength = $max;
            return $field;
        })()`;
    };
    const placeholderEmit = (id, text, url) =>
        `$emit({t:"placeholder",id:${jsString(id)},text:${jsString(text)}${url ? `,url:${url}` : ""}});\n`;
    const emitWidget = (field, sub, widget) => {
        if (widget.name === "link") return linkStatement(widget, false);
        const expr = fieldObject(field, sub, widget);
        if (!expr) return placeholderEmit(fieldIdOf(field) + "." + widget.name, widget.name);
        return `$emit(${expr});\n`;
    };
    const emitSub = (field, sub) => {
        let code = S.setCode(sub);
        sub.children.forEach((widget) => {
            if (widget.name === "set" || widget.name === "ignored") return;
            code += emitWidget(field, sub, widget);
        });
        return code;
    };
    const emitField = (field) => {
        if (!field || attr(field, "hide") === "true") return "";
        return S.fieldChain(field, (sub) => emitSub(field, sub));
    };
    const layoutCode = (form) => {
        const fields = childrenNamed(form, "field");
        const layout = child(form, "field-layout");
        if (!layout) return fields.map(emitField).join("");
        const byName = {};
        fields.forEach((field) => { byName[attr(field, "name")] = field; });
        const used = new Set();
        let seq = 0;
        const walk = (node) => {
            let code = "";
            node.children.forEach((kid) => {
                if (kid.name === "field-ref") {
                    const name = attr(kid, "name");
                    used.add(name);
                    code += emitField(byName[name]);
                } else if (kid.name === "fields-not-referenced") {
                    fields.forEach((field) => {
                        const name = attr(field, "name");
                        if (!used.has(name)) {
                            used.add(name);
                            code += emitField(field);
                        }
                    });
                } else if (kid.name === "field-row" || kid.name === "field-row-big" || kid.name === "field-col-row") {
                    const inner = walk(kid);
                    code += `$openChildren({t:"row",id:${jsString(formNameOf(form) + ".l" + seq++)}});\n${inner}$closeChildren();\n`;
                } else if (kid.name === "field-col") {
                    const inner = walk(kid);
                    code += `$openChildren({t:"col",id:${jsString(formNameOf(form) + ".l" + seq++)},xs:${jsString(attr(kid, "xs"))},sm:${jsString(attr(kid, "sm"))},md:${jsString(attr(kid, "md"))},lg:${jsString(attr(kid, "lg"))}});\n${inner}$closeChildren();\n`;
                } else if (kid.name === "field-group" || kid.name === "field-accordion") {
                    const inner = walk(kid);
                    code += `$openChildren({t:"box",id:${jsString(formNameOf(form) + ".l" + seq++)},title:${exprOf(attr(kid, "title"))},type:${jsString(kid.name)}});\n${inner}$closeChildren();\n`;
                }
            });
            return code;
        };
        return walk(layout);
    };

    const cellText = (widget) => {
        if (widget.name === "password") return '""';
        const field = fieldNodeOf(widget);
        if (field && SECRET.has((attr(field, "name") || "").toLowerCase())) return '""';
        if (widget.name === "link") {
            const expr = linkExpr(widget, true);
            const condition = attr(widget, "condition");
            return condition ? `(() => { if (!(${condition})) return ""; return ${expr}; })()` : expr;
        }
        if (widget.name === "hidden" || widget.name === "ignored") return '""';
        if (widget.name === "drop-down" && child(widget, "dynamic-options")) {
            const dynamic = child(widget, "dynamic-options");
            return `(() => {
                const $value = String(${valueCall(widget)} || "");
                if (!$value) return "";
                const $found = sri.getFieldTransitionValue(${jsString(attr(dynamic, "transition") || "")}, ${S.nodeLit(dynamic, true)}, $value, ${jsString(attr(dynamic, "label-field") || "label")}, false);
                return $found ? String($found) : $value;
            })()`;
        }
        if (widget.name === "drop-down" || widget.name === "radio" || widget.name === "check") {
            return `(() => {
                const $value = String(${valueCall(widget)} || "");
                const $map = sri.getFieldOptions(${S.nodeLit(widget, true)});
                const $label = $map && $map.get ? $map.get($value) : null;
                return $label != null ? String($label) : $value;
            })()`;
        }
        if (widget.name === "date-time") {
            return `String(sri.getFieldValueString(${S.fieldLit(widget)}, ${jsString(attr(widget, "default-value"))}, ${jsString(S.dateTimeFormat(widget))}, ${jsString(attr(widget, "type") || "date-time")}) || "")`;
        }
        if (widget.name === "display") return `String(${S.displayValueExpr(widget)} || "")`;
        if (widget.name === "display-entity") return `String(sri.getFieldEntityValue(${S.nodeLit(widget, false)}) || "")`;
        if (widget.name === "text-line" || widget.name === "text-area" || widget.name === "text-find") {
            return `String(${valueCall(widget)} || "")`;
        }
        if (widget.name === "submit" || widget.name === "reset") return `String(${S.localizedTitle(widget.parent)} || "")`;
        return jsString(widget.name);
    };
    const columnCells = (columns) => {
        let code = `let $cell = "";\nlet $piece = "";\n`;
        columns.forEach((column) => {
            code += `$cell = "";\n$piece = "";\n`;
            column.forEach((field) => {
                if (attr(field, "hide") === "true") return;
                const body = S.fieldChain(field, (sub) => {
                    let inner = S.setCode(sub);
                    sub.children.forEach((widget) => {
                        if (widget.name === "set" || widget.name === "ignored") return;
                        inner += `$piece = ${cellText(widget)};\n`;
                        inner += `if ($piece && typeof $piece === "object") $cell = $piece;\n`;
                        inner += `else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);\n`;
                    });
                    return inner;
                });
                code += S.hideGuard(field, body);
            });
            code += `$cells.push($cell);\n`;
        });
        return code;
    };
    const headerFields = (form) => {
        let code = "";
        childrenNamed(form, "field").forEach((field) => {
            const sub = child(field, "header-field");
            if (!sub || attr(field, "hide") === "true") return;
            code += S.setCode(sub);
            sub.children.forEach((widget) => {
                if (widget.name === "set" || widget.name === "ignored") return;
                if (widget.name === "link") {
                    code += `$headerFields.push(${linkExpr(widget, false)});\n`;
                    return;
                }
                const expr = fieldObject(field, sub, widget);
                code += expr
                    ? `$headerFields.push(${expr});\n`
                    : `$headerFields.push({t:"placeholder",id:${jsString(fieldIdOf(field) + "." + widget.name)},text:${jsString(widget.name)}});\n`;
            });
        });
        return code;
    };

    const formSingle = stmt((node) => {
        const name = formNameOf(node);
        const id = attr(node, "id") || name;
        const transition = attr(node, "transition") || "";
        const map = attr(node, "map") || "fieldValues";
        const focus = attr(node, "focus-field") || "";
        const body = layoutCode(node);
        const transitionCode = transition
            ? `const $made = sri.makeUrlByType(${jsString(transition)}, "transition", ${S.nodeLit(node, false)}, "true");\n$transitionUrl = $made ? ($made.url || "") : "";\n`
            : "";
        const focusCode = focus
            ? `const $focusName = ec.resource.expand(${jsString(focus)}, "");\nif ($focusName && $focusName !== "null") sri.setFocusField(${jsString(name)} + "." + $focusName);\n`
            : "";
        return `sri.pushSingleFormMapContext(${map});
try {
    const $hidden = [];
    const $hiddenMap = sri.getFormHiddenParameters(${S.nodeLit(node, true)}) || {};
    const $hiddenKeys = Object.keys($hiddenMap);
    for (let $hi = 0; $hi < $hiddenKeys.length; $hi++) {
        const $key = $hiddenKeys[$hi];
        const $value = $hiddenMap[$key];
        if ($value == null || $value === "") continue;
        if ($secret.has(String($key).toLowerCase())) continue;
        $hidden.push({name:String($key),value:String($value)});
    }
    let $transitionUrl = "";
    ${transitionCode}${focusCode}$openChildren({t:"form",id:${jsString(id)},name:${jsString(name)},kind:"single",transitionUrl:$transitionUrl,method:${jsString(transition ? "post" : "")},hidden:$hidden,submitMessage:${attr(node, "submit-message") ? S.expandExpr(attr(node, "submit-message")) : '""'},backgroundReloadId:${exprOf(attr(node, "background-reload-id") || "")},backgroundHideId:${exprOf(attr(node, "background-hide-id") || "")}});
    ${body}$closeChildren();
} finally {
    sri.popContext();
}
`;
    });

    const formList = stmt((node) => {
        const name = formNameOf(node);
        const id = attr(node, "id") || name;
        const listName = attr(node, "list") || "";
        const listKey = /^[A-Za-z_][A-Za-z0-9_]*$/.test(listName) ? listName : "";
        const idField = attr(node, "id-field") || "";
        const columns = S.formListColumns(node);
        const titles = columns.map((column) => {
            const parts = column.map((field) => {
                const sub = S.headerSub(field);
                return sub ? `String(${S.localizedTitle(sub)} || "")` : jsString(attr(field, "name") || "");
            });
            // orderBy is the orderByField value for the column's first field with show-order-by, as the html header links send it
            const showOrderBy = (field) => {
                const header = child(field, "header-field");
                return header ? attr(header, "show-order-by") : "";
            };
            const sortField = column.find((field) => showOrderBy(field) === "true" || showOrderBy(field) === "case-insensitive");
            const orderBy = sortField ? (showOrderBy(sortField) === "case-insensitive" ? "^" : "") + attr(sortField, "name") : "";
            const orderCode = orderBy ? `,orderBy:${jsString(orderBy)}` : "";
            return `$columns.push({title:[${parts.join(",")}].filter(($part) => $part).join(" / ")${orderCode}});\n`;
        }).join("");
        const rowKey = idField
            ? `String(sri.getNamedValuePlain(${jsString(idField)}, ${S.nodeLit(node, false)}) || $index)`
            : "String($index)";
        const pageName = (suffix) => (listKey ? jsString(listKey + suffix) : '""');
        return `{
    const $info = sri.openFormList(${jsString(name)});
    try {
        const $columns = [];
        ${titles}const $headerFields = [];
        ${headerFields(node)}const $rows = [];
        for (let $index = 0; $index < $info.count; $index++) {
            sri.startFormListRow($info, $index);
            try {
                const $rowKey = ${rowKey};
                const $cells = [];
                ${columnCells(columns)}$rows.push({cells:$cells});
            } finally {
                sri.endFormListRow();
            }
        }
        const $indexValue = ${pageName("PageIndex")} ? ec.context.get(${pageName("PageIndex")}) : null;
        const $pageIndex = $indexValue == null || $indexValue === "" ? $whole(ec.context.get("pageIndex")) : $whole($indexValue);
        const $sizeValue = ${pageName("PageSize")} ? ec.context.get(${pageName("PageSize")}) : null;
        const $pageSize = $sizeValue == null || $sizeValue === "" ? $whole(ec.context.get("pageSize")) : $whole($sizeValue);
        const $total = ${pageName("Count")} ? ec.context.get(${pageName("Count")}) : null;
        const $count = $total == null || $total === "" ? $info.count : $whole($total);
        const $size = $pageSize > 0 ? $pageSize : $info.count;
        $emit({t:"formList",id:${jsString(id)},name:${jsString(name)},columns:$columns,rows:$rows,paging:{index:$pageIndex,size:$size,count:$count},headerFields:$headerFields});
    } finally {
        sri.safeCloseList($info);
    }
}
`;
    });

    const rawText = (node) => {
        if (attr(node, "location")) {
            return `sri.renderText(${S.expandNoL10nExpr(attr(node, "location"))}, ${jsString(attr(node, "template") || "true")})`;
        }
        const inline = node.text || "";
        const template = attr(node, "template");
        return !template || template === "true" ? `sri.renderInlineText(${jsString(inline)})` : jsString(inline);
    };
    const tuiText = (node) => {
        if (!attr(node, "location") && !(node.text || "").trim()) return "";
        const where = `String((sri.getActiveScreenDef() && sri.getActiveScreenDef().location) || "") + " " + ${jsString(attr(node, "location") || "inline")} + " is not JSON"`;
        return `{
    const $rawValue = ${rawText(node)};
    const $raw = String($rawValue == null ? "" : $rawValue);
    let $parsed = null;
    try { $parsed = JSON.parse($raw); } catch ($parseError) { $parsed = null; }
    if (Array.isArray($parsed)) $emit({t:"fragment",id:${jsString(idOf(node))},c:$parsed});
    else $emit({t:"error",id:${jsString(idOf(node))},text:${where}});
}
`;
    };
    const anyText = (node) => {
        if (!attr(node, "location") && !(node.text || "").trim()) return "";
        return `{
    const $rawValue = ${rawText(node)};
    $emit({t:"text",id:${jsString(idOf(node))},text:${S.htmlViewTextFn}(String($rawValue == null ? "" : $rawValue), "")});
}
`;
    };
    const textCode = (node) => {
        const type = attr(node, "type");
        if (typeNames(type).includes("tui")) return tuiText(node);
        if (!type || type === "any") return anyText(node);
        return `$emit({t:"placeholder",id:${jsString(idOf(node))},text:"text",url:${screenUrlExpr}});\n`;
    };
    const menuItems = `const $menu = sri.getMenuItems() || [];
        const $items = [];
        for (let $mi = 0; $mi < $menu.length; $mi++) {
            const $item = $menu[$mi];
            $items.push({title:$item.title || "",url:$item.url || "",active:!!$item.inPath});
        }
`;

    const pass = stmt((node, children) => children);
    const group = (type, extra) => stmt((node, children) =>
        `$openChildren({t:${jsString(type)},id:${jsString(idOf(node))}${extra ? extra(node) : ""}});\n${children}$closeChildren();\n`);

    const overrides = {
        "screen": pass,
        "widgets": pass,
        "fail-widgets": pass,
        "dynamic-form": pass,
        "form-list-user-columns": stmt((node, children) =>
            `if (!sri.renderUserColumnsForm(${jsString(attr(node, "name") || "list")})) {\n${children}}\n`),
        "box-header": pass,
        "box-toolbar": pass,
        "box-body": pass,
        "box-body-nopad": pass,
        "panel-header": pass,
        "panel-left": pass,
        "panel-center": pass,
        "panel-right": pass,
        "panel-footer": pass,
        "split-side": pass,
        "split-main": pass,
        "field": pass,
        "header-field": pass,
        "first-row-field": pass,
        "second-row-field": pass,
        "last-row-field": pass,
        "default-field": pass,
        "conditional-field": pass,
        "field-group": pass,
        "field-accordion": pass,
        "field-row": pass,
        "field-row-big": pass,
        "field-col-row": pass,
        "field-col": pass,
        "fields-not-referenced": pass,
        "subscreens-menu": pass,
        "container": group("container"),
        "container-box": group("box", (node) => {
            const header = child(node, "box-header");
            const title = header ? attr(header, "title") : "";
            return `,title:${exprOf(title)},type:${jsString(attr(node, "type"))},initial:${jsString(attr(node, "initial"))}`;
        }),
        "container-row": group("row"),
        "row-col": group("col", (node) =>
            `,xs:${jsString(attr(node, "xs"))},sm:${jsString(attr(node, "sm"))},md:${jsString(attr(node, "md"))},lg:${jsString(attr(node, "lg"))}`),
        "container-panel": group("panel"),
        "container-dialog": stmt((node, children) => {
            const button = attr(node, "button-text") || "";
            const title = attr(node, "title") || button;
            return guarded(node, `$openChildren({t:"dialog",id:${jsString(idOf(node))},title:${exprOf(title)},button:${exprOf(button)}});\n${children}$closeChildren();\n`);
        }),
        "container-split": group("split"),
        "dashboard": group("dashboard"),
        "section": stmt((node) =>
            `$openRender({t:"section",id:${jsString(idOf(node))},name:${jsString(attr(node, "name") || "")}});\nsri.renderSection(${jsString(attr(node, "name") || "")});\n$closeRender();\n`),
        "section-iterate": stmt((node) =>
            `$openRender({t:"section",id:${jsString(idOf(node))},name:${jsString(attr(node, "name") || "")},iterate:true});\nsri.renderSection(${jsString(attr(node, "name") || "")});\n$closeRender();\n`),
        "section-include": stmt((node) =>
            `$openRender({t:"section",id:${jsString(idOf(node))},name:${jsString(attr(node, "name") || "")}});\nsri.renderSectionInclude(${S.bareLit(node)});\n$closeRender();\n`),
        "subscreens-active": stmt((node) =>
            `$openRender({t:"active",id:${jsString(attr(node, "id") || "subscreens")}});\nsri.renderSubscreen();\n$closeRender();\n`),
        "subscreens-panel": stmt((node) => {
            const type = attr(node, "type") || "tab";
            const id = attr(node, "id") || "subscreens";
            if (type === "popup") return "";
            if (type === "wizard" || type === "stack") {
                return `{
                    ${menuItems}let $step = ${exprOf(attr(node, "title") || "")};
                    for (let $si = 0; $si < $items.length; $si++) if ($items[$si].active && $items[$si].title) $step = $items[$si].title;
                    $emit({t:"text",id:${jsString(id + "-step")},text:String($step || "")});
                    $openRender({t:"active",id:${jsString(id)}});
                    sri.renderSubscreen();
                    $closeRender();
                }
`;
            }
            return `{
                ${menuItems}$openRender({t:"subscreens",id:${jsString(id)},kind:${jsString(type)},items:$items});
                sri.renderSubscreen();
                $closeRender();
            }
`;
        }),
        "include-screen": stmt((node) =>
            `$openRender({t:"section",id:${jsString(attr(node, "location") || "include")},name:"include"});\nsri.renderIncludeScreen(${jsString(attr(node, "location"))}, ${jsString(attr(node, "share-scope"))});\n$closeRender();\n`),
        "render-dynamic-form": stmt((node) => {
            const name = attr(node, "name") || "form";
            return `$openRender({t:"fragment",id:${jsString(name)}});\nsri.renderDynamicForm(${jsString(name)});\n$closeRender();\n`;
        }),
        "render-mode": stmt((node) => {
            const texts = childrenNamed(node, "text");
            const tui = texts.find((kid) => typeNames(attr(kid, "type")).includes("tui"));
            if (tui) return textCode(tui);
            const any = texts.find((kid) => !attr(kid, "type") || attr(kid, "type") === "any");
            if (any) return textCode(any);
            return `$emit({t:"placeholder",id:${jsString(idOf(node))},text:"render-mode",url:${screenUrlExpr}});\n`;
        }),
        "text": stmt((node) => textCode(node)),
        "label": stmt((node) => guarded(node,
            `$emit({t:"text",id:${jsString(idOf(node))},text:${S.expandExpr(attr(node, "text") || "")}});\n`)),
        "link": stmt((node) => {
            if (fieldNodeOf(node) && formNodeOf(node)) return "";
            return linkStatement(node, inHeader(node) ? false : formNodeOf(node) && formNodeOf(node).name === "form-list");
        }),
        "form-single": formSingle,
        "form-list": formList,
        "number-card": stmt((node) => guarded(node,
            `$emit({t:"numberCard",id:${jsString(idOf(node))},title:String(${S.expandExpr(attr(node, "title") || "")} || ""),value:String(${S.expandNoL10nExpr(attr(node, "value") || "")} || ""),url:String(${S.expandNoL10nExpr(attr(node, "url") || "")} || "")});\n`)),
        "dynamic-dialog": stmt((node) => guarded(node, `{
    const $made = sri.makeUrlByType(${jsString(attr(node, "transition") || "")}, "transition", ${S.nodeLit(node, false)}, "true");
    $emit({t:"dynamicDialog",id:${jsString(idOf(node))},url:$made ? ($made.url || "") : "",button:String(${S.expandExpr(attr(node, "button-text") || "")} || "")});
}
`)),
        "dynamic-container": stmt((node) => `{
    const $made = sri.makeUrlByType(${jsString(attr(node, "transition") || "")}, "transition", ${S.nodeLit(node, false)}, "true");
    $emit({t:"dynamicContainer",id:${jsString(idOf(node))},url:$made ? ($made.url || "") : ""});
}
`),
        "image": stmt((node) =>
            `$emit({t:"image",id:${jsString(idOf(node))},alt:String(${S.expandExpr(attr(node, "alt") || "")} || ""),url:String(${S.expandNoL10nExpr(attr(node, "url") || "")} || "")});\n`),
        "markdown-view": stmt((node) => guarded(node,
            `$emit({t:"markdown",id:${jsString(idOf(node))},text:String(${S.markdownTextExpr(node)} || "")});\n`)),
        "status-bar": stmt((node) => guarded(node,
            `$emit({t:"statusBar",id:${jsString(idOf(node))},text:String(${S.statusTextExpr(node)} || "")});\n`)),
        "html-view": stmt((node) => guarded(node,
            `$emit({t:"text",id:${jsString(idOf(node))},text:String(${S.htmlViewTextExpr(node)} || "")});\n`)),
        "button-menu": stmt((node) =>
            `$emit({t:"buttonMenu",id:${jsString(idOf(node))},text:String(${S.expandExpr(attr(node, "text") || "")} || "")});\n`),
        "tree": stmt((node) => `$emit({t:"tree",id:${jsString(attr(node, "id") || attr(node, "name") || "tree")}});\n`),
        "include-content": stmt((node) =>
            `$emit({t:"placeholder",id:${jsString(idOf(node))},text:"include-content",url:String(${S.expandNoL10nExpr(attr(node, "location") || "")} || "")});\n`)
    };

    const macros = {};
    S.allElements.forEach((name) => {
        macros[name] = stmt((node) => placeholderEmit(idOf(node), node.name));
    });
    Object.assign(macros, overrides);

    return { macros, opaque: S.opaqueElements, prelude, epilogue };
})
