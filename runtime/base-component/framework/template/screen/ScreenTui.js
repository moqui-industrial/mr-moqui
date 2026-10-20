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

    // Field and parameter names never written, compared without case: CREDENTIAL_PARAMETER_NAMES in
    // crates/moqui-core/src/web/mod.rs. Field names are checked as the code is made, hidden parameter names as it runs.
    const SECRET = new Set(["password", "oldpassword", "newpassword", "newpasswordverify", "currentpassword", "resetpassword", "authpassword", "api_key", "login_key", "passwordsalt", "passwordhashtype", "passwordbase64", "passwordverify"]);
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
            out += JSON.stringify($clean(key)) + ":" + $scrub(value[key]);
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
const $uid = (id) => sri.uniqueId(String(id));
const $secret = new Set(${JSON.stringify([...SECRET])});
sri.write("[");
`;
    const epilogue = `
sri.write("]");
`;

    const typeNames = (type) => (type || "").split(",").map((name) => name.trim()).filter((name) => name);
    const exprOf = (text) => (text && String(text).includes("${") ? S.expandExpr(text) : jsString(text || ""));
    // A title as the other modes show it: expanded and localized.
    const titleOf = (text) => (text ? S.expandExpr(text) : '""');
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
            const $link = {t:"link",id:$uid(${linkIdExpr(node, inList)}),text:String($text),url:$made ? ($made.getUrlWithParams() || "") : ""};
            if ($made && $made.getDisableLink()) $link.disabled = true;
            ${kind ? `$link.kind = ${jsString(kind)};` : ""}
            ${attr(node, "btn-type") ? `$link.btnType = ${jsString(attr(node, "btn-type"))};` : ""}
            ${attr(node, "icon") ? `$link.icon = ${jsString(attr(node, "icon"))};` : ""}
            ${attr(node, "badge") ? `$link.badge = String(${S.expandNoL10nExpr(attr(node, "badge"))} || "");` : ""}
            ${attr(node, "tooltip") ? `$link.tooltip = String(${S.expandExpr(attr(node, "tooltip"))} || "");` : ""}
            ${kind === "hidden-form-link" ? `$link.method = "post";` : ""}
            ${confirm ? `$link.confirm = String(${titleOf(confirm)} || "");` : ""}
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
    // As qvt's remote options: the URL without its query, its fixed parameters, and {parameter: field} for depends-on.
    const dynamicOptions = (widget) => {
        const dynamic = child(widget, "dynamic-options");
        const depends = {};
        childrenNamed(dynamic, "depends-on").forEach((dep) => {
            const field = attr(dep, "field");
            if (field) depends[attr(dep, "parameter") || field] = field;
        });
        const labelField = attr(dynamic, "label-field") || "label";
        return `(() => {
            const $value = String(${valueCall(widget)} || "");
            const $made = sri.makeUrlByType(${jsString(attr(dynamic, "transition") || "")}, "transition", ${S.nodeLit(dynamic, true)}, "false");
            const $optionsUrl = $made ? ($made.url || "") : "";
            const $depends = ${JSON.stringify(depends)};
            const $params = {};
            const $map = $made ? ($made.getParameterMap() || {}) : {};
            for (const $key of Object.keys($map)) {
                const $fixed = $map[$key];
                if ($depends[$key] === undefined && $fixed != null && $fixed !== "") $params[$key] = String($fixed);
            }
            let $label = $value;
            if ($value) {
                const $found = sri.getFieldTransitionValue(${jsString(attr(dynamic, "transition") || "")}, ${S.nodeLit(dynamic, true)}, $value, ${jsString(labelField)}, false);
                if ($found) $label = String($found);
            }
            const $w = {kind:"dropDown",value:$value,options:$value ? [{value:$value,label:$label}] : []};
            if ($optionsUrl) $w.optionsUrl = $optionsUrl;
            if (Object.keys($depends).length) $w.dependsOn = $depends;
            if (Object.keys($params).length) $w.optionsParameters = $params;
            ${attr(dynamic, "server-search") === "true" ? "$w.serverSearch = true;\n            " : ""}return $w;
        })()`;
    };
    // A find value from the request: `name` plus a suffix (`_from`, `_op`), else the default, as qvt's fields-initial.
    const findValue = (field, suffix, fallback) =>
        `(() => { const $v = ec.context.get(${jsString((attr(field, "name") || "") + suffix)}); return $v == null || $v === "" || $v === "null" ? String(${fallback || '""'}) : String($v); })()`;
    const dateDefault = (widget) => (attr(widget, "type") === "time" ? "HH:mm" : attr(widget, "type") === "date" ? "yyyy-MM-dd" : "yyyy-MM-dd HH:mm");
    const flag = (widget, name) => attr(widget, name) === "true";
    const widgetExpr = (widget) => {
        const field = fieldNodeOf(widget);
        const secret = field && SECRET.has((attr(field, "name") || "").toLowerCase());
        if (widget.name === "password") return '{kind:"password"}';
        if (secret) {
            if (widget.name === "display" || widget.name === "display-entity") return '{kind:"display",text:""}';
            const kind = widget.name === "text-find" ? "textFind" : widget.name === "hidden" ? "hidden" : "textLine";
            return `{kind:${jsString(kind)},value:""}`;
        }
        if (widget.name === "text-line") {
            const extra = [
                attr(widget, "input-type") ? `inputType:${jsString(attr(widget, "input-type"))}` : "",
                attr(widget, "mask") ? `mask:${jsString(attr(widget, "mask"))}` : "",
                /^\d+$/.test(attr(widget, "size")) ? `size:${attr(widget, "size")}` : ""
            ].filter(Boolean).map((part) => "," + part).join("");
            return `{kind:"textLine",value:String(${valueCall(widget)} || "")${extra}}`;
        }
        if (widget.name === "text-area") {
            // D12: rich text is shown, never posted, so the stored HTML is not overwritten from the terminal.
            const editor = attr(widget, "editor-type");
            const readOnly = flag(widget, "read-only") || ["html", "rich", "rich-light"].includes(editor);
            const rows = /^\d+$/.test(attr(widget, "rows")) ? attr(widget, "rows") : "0";
            return `{kind:"textArea",value:String(${valueCall(widget)} || ""),rows:${rows},editorType:${jsString(editor)}${readOnly ? ",readOnly:true" : ""}}`;
        }
        if (widget.name === "text-find") {
            const ignoreCase = !attr(widget, "ignore-case") || flag(widget, "ignore-case");
            return `{kind:"textFind",value:String(${valueCall(widget)} || ""),op:${findValue(field, "_op", jsString(attr(widget, "default-operator") || "contains"))},not:${findValue(field, "_not", '"N"')} === "Y",ic:${findValue(field, "_ic", jsString(ignoreCase ? "Y" : "N"))} === "Y"${flag(widget, "hide-options") ? ",hideOptions:true" : ""}}`;
        }
        if (widget.name === "drop-down") {
            const base = child(widget, "dynamic-options") ? dynamicOptions(widget) : inlineOptions(widget, "dropDown");
            const flags = [
                ["allow-multiple", "multiple"], ["combo-box", "combo"], ["submit-on-select", "submitOnSelect"], ["show-not", "showNot"]
            ].filter(([name]) => attr(widget, name) === "true").map(([, key]) => `$w.${key} = true;`);
            if (attr(widget, "allow-empty") === "true") flags.push("$w.allowEmpty = true;");
            if (attr(widget, "show-not") === "true") flags.push(`$w.not = ${findValue(field, "_not", '"N"')} === "Y";`);
            if (!flags.length) return base;
            return `(() => { const $w = ${base}; ${flags.join(" ")} return $w; })()`;
        }
        if (widget.name === "radio") return inlineOptions(widget, "radio");
        if (widget.name === "check") return inlineOptions(widget, "check");
        if (widget.name === "date-time") {
            const type = attr(widget, "type") || "date-time";
            return `{kind:"dateTime",value:String(sri.getFieldValueString(${S.fieldLit(widget)}, ${jsString(attr(widget, "default-value"))}, ${jsString(S.dateTimeFormat(widget))}, ${jsString(type)}) || ""),type:${jsString(type)},format:${jsString(S.dateTimeFormat(widget))}}`;
        }
        if (widget.name === "date-find") {
            const format = dateDefault(widget);
            const formatted = (suffix, fallback) => `(() => { const $raw = ${findValue(field, suffix, jsString(fallback))}; if (!$raw) return ""; try { return String(ec.l10n.format($raw, ${jsString(format)})); } catch ($err) { return $raw; } })()`;
            return `{kind:"dateFind",type:${jsString(attr(widget, "type") || "timestamp")},format:${jsString(format)},from:${formatted("_from", attr(widget, "default-value-from"))},thru:${formatted("_thru", attr(widget, "default-value-thru"))}}`;
        }
        if (widget.name === "date-period") {
            return `{kind:"datePeriod",period:${findValue(field, "_period")}.toLowerCase(),poffset:${findValue(field, "_poffset")},pdate:${findValue(field, "_pdate")},from:${findValue(field, "_from")},thru:${findValue(field, "_thru")},allowEmpty:${attr(widget, "allow-empty") !== "false"},time:${flag(widget, "time")}}`;
        }
        if (widget.name === "range-find") {
            return `{kind:"rangeFind",from:${findValue(field, "_from", jsString(attr(widget, "default-value-from")))},thru:${findValue(field, "_thru", jsString(attr(widget, "default-value-thru")))}}`;
        }
        if (widget.name === "file") return `{kind:"file"${flag(widget, "multiple") ? ",multiple:true" : ""}${attr(widget, "accept") ? `,accept:${jsString(attr(widget, "accept"))}` : ""}}`;
        if (widget.name === "display" || widget.name === "display-entity") {
            const text = widget.name === "display" ? S.displayValueExpr(widget) : `sri.getFieldEntityValue(${S.nodeLit(widget, false)})`;
            // also-hidden (the default) posts the plain value with the form, as qvt's display does.
            const alsoHidden = !attr(widget, "also-hidden") || flag(widget, "also-hidden");
            return `{kind:"display",text:String(${text} || "")${alsoHidden ? `,value:String(sri.getFieldValuePlainString(${S.fieldLit(widget)}, "") || "")` : ""}}`;
        }
        if (widget.name === "hidden") return `{kind:"hidden",value:String(${valueCall(widget)} || "")}`;
        if (widget.name === "submit" || widget.name === "reset") {
            return `{kind:${jsString(widget.name)},text:String(${S.localizedTitle(widget.parent)} || "")${widget.name === "submit" && attr(widget, "confirmation") ? `,confirm:String(${titleOf(attr(widget, "confirmation"))} || "")` : ""}}`;
        }
        if (widget.name === "label") {
            return `{kind:"label",text:String(${S.expandExpr(attr(widget, "text") || "")} || ""),style:${jsString(attr(widget, "type") || "span")}}`;
        }
        if (widget.name === "image") {
            return `{kind:"image",alt:String(${S.expandExpr(attr(widget, "alt") || "")} || ""),url:String(${S.expandNoL10nExpr(attr(widget, "url") || "")} || "")}`;
        }
        return "";
    };
    const fieldObject = (field, sub, widget) => {
        const w = widgetExpr(widget);
        if (!w) return "";
        const when = child(sub, "visible-when");
        const required = attr(field, "required") === "true" || attr(sub, "required") === "true";
        return `(() => {
            let $required = ${required ? "true" : "false"};
            const $max = ${maxlengthOf(sub, widget)};
            const $hint = sri.getFieldValidation(${S.nodeLit(sub, false)});
            if ($hint && String($hint.classes || "").split(/\\s+/).indexOf("required") >= 0) $required = true;
            const $rules = [];
            const $found = sri.getFieldValidationRules(${S.nodeLit(sub, false)}) || [];
            for (let $ri = 0; $ri < $found.length; $ri++) {
                if ($found[$ri].kind === "required") $required = true;
                else $rules.push($found[$ri]);
            }
            const $field = {t:"field",id:$uid(${jsString(fieldIdOf(field))}),name:${jsString(attr(field, "name") || "field")},title:String(${S.localizedTitle(sub)} || ""),w:${w}};
            if ($required) $field.required = true;
            if ($max > 0) $field.maxlength = $max;
            if ($rules.length) $field.rules = $rules;
            ${attr(sub, "tooltip") ? `$field.tooltip = String(${S.expandExpr(attr(sub, "tooltip"))} || "");` : ""}
            ${when ? `$field.visibleWhen = {field:${jsString(attr(when, "field"))},value:String(${exprOf(attr(when, "value"))} || "")};` : ""}
            return $field;
        })()`;
    };
    // Children of a field sub-node that are not drawn: set, ignored, and the visible-when rule the field carries.
    const notWidget = (widget) => widget.name === "set" || widget.name === "ignored" || widget.name === "visible-when";
    const placeholderEmit = (id, text, url) =>
        `$emit({t:"placeholder",id:$uid(${jsString(id)}),text:${jsString(text)}${url ? `,url:${url}` : ""}});\n`;
    const emitWidget = (field, sub, widget) => {
        if (widget.name === "link") return linkStatement(widget, false);
        const expr = fieldObject(field, sub, widget);
        if (!expr) return placeholderEmit(fieldIdOf(field) + "." + widget.name, widget.name);
        return `$emit(${expr});\n`;
    };
    const emitSub = (field, sub) => {
        let code = S.setCode(sub);
        sub.children.forEach((widget) => {
            if (notWidget(widget)) return;
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
                    code += `$openChildren({t:"row",id:$uid(${jsString(formNameOf(form) + ".l" + seq++)})});\n${inner}$closeChildren();\n`;
                } else if (kid.name === "field-col") {
                    const inner = walk(kid);
                    code += `$openChildren({t:"col",id:$uid(${jsString(formNameOf(form) + ".l" + seq++)}),xs:${jsString(attr(kid, "xs"))},sm:${jsString(attr(kid, "sm"))},md:${jsString(attr(kid, "md"))},lg:${jsString(attr(kid, "lg"))}});\n${inner}$closeChildren();\n`;
                } else if (kid.name === "field-group" || kid.name === "field-accordion") {
                    const inner = walk(kid);
                    code += `$openChildren({t:"box",id:$uid(${jsString(formNameOf(form) + ".l" + seq++)}),title:${titleOf(attr(kid, "title"))},type:${jsString(kid.name)}});\n${inner}$closeChildren();\n`;
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
                        if (notWidget(widget)) return;
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
                if (notWidget(widget)) return;
                if (widget.name === "link") {
                    code += `$headerFields.push(${linkExpr(widget, false)});\n`;
                    return;
                }
                const expr = fieldObject(field, sub, widget);
                code += expr
                    ? `$headerFields.push(${expr});\n`
                    : `$headerFields.push({t:"placeholder",id:$uid(${jsString(fieldIdOf(field) + "." + widget.name)}),text:${jsString(widget.name)}});\n`;
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
    ${transitionCode}${focusCode}$openChildren({t:"form",id:$uid(${jsString(id)}),name:${jsString(name)},kind:"single",transitionUrl:$transitionUrl,method:${jsString(transition ? "post" : "")},hidden:$hidden,submitMessage:${attr(node, "submit-message") ? S.expandExpr(attr(node, "submit-message")) : '""'},backgroundReloadId:${exprOf(attr(node, "background-reload-id") || "")},backgroundHideId:${exprOf(attr(node, "background-hide-id") || "")}});
    ${body}$closeChildren();
} finally {
    sri.popContext();
}
`;
    });

    // A column of numbers: a currency or number-formatted display, or a number input.
    const numeric = (field) => {
        const sub = child(field, "default-field");
        if (!sub) return false;
        return sub.children.some((widget) =>
            (widget.name === "display" && (attr(widget, "currency-unit-field") || /[#0]/.test(attr(widget, "format")))) ||
            (widget.name === "text-line" && attr(widget, "input-type") === "number"));
    };
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
            // The client sizes columns from print-width as the text mode does, against the canvas it has.
            const sized = column.find((field) => attr(field, "print-width"));
            const widthCode = sized ? `,printWidth:${jsString(attr(sized, "print-width"))},printWidthType:${jsString(attr(sized, "print-width-type") || "characters")}` : "";
            const alignCode = column.some(numeric) ? `,align:"right"` : "";
            return `$columns.push({title:[${parts.join(",")}].filter(($part) => $part).join(" / ")${orderCode}${widthCode}${alignCode}});\n`;
        }).join("");
        const rowKey = idField
            ? `String(sri.getNamedValuePlain(${jsString(idField)}, ${S.nodeLit(node, false)}) || $index)`
            : "String($index)";
        const pageName = (suffix) => (listKey ? jsString(listKey + suffix) : '""');
        const selection = child(node, "row-selection");
        const selectIdField = selection ? attr(selection, "id-field") : "";
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
                ${columnCells(columns)}const $row = {id:$rowKey,cells:$cells};
                ${selectIdField ? `const $selectId = ec.context.get(${jsString(selectIdField)});
                if ($selectId != null && $selectId !== "") $row.selectId = String($selectId);
                ` : ""}$rows.push($row);
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
        const $paging = {index:$pageIndex,size:$size,count:$count};
        for (const [$key, $suffix] of [["maxIndex", "PageMaxIndex"], ["rangeLow", "PageRangeLow"], ["rangeHigh", "PageRangeHigh"]]) {
            const $value = ${listKey ? `ec.context.get(${jsString(listKey)} + $suffix)` : "null"};
            if ($value != null && $value !== "") $paging[$key] = $whole($value);
        }
        const $list = {t:"formList",id:$uid(${jsString(id)}),name:${jsString(name)},columns:$columns,rows:$rows,paging:$paging,headerFields:$headerFields};
        ${selection ? `$list.rowSelection = {idField:${jsString(selectIdField)},parameter:${jsString(attr(selection, "parameter") || selectIdField)}${attr(selection, "list-mode") ? `,listMode:${jsString(attr(selection, "list-mode"))}` : ""}};
        ` : ""}$emit($list);
    } finally {
        sri.safeCloseList($info);
    }
}
`;
    });

    const rawText = (node) => {
        if (attr(node, "location")) {
            return `sri.renderTextValue(${S.expandNoL10nExpr(attr(node, "location"))}, ${jsString(attr(node, "template") || "true")})`;
        }
        const inline = node.text || "";
        const template = attr(node, "template");
        return !template || template === "true" ? `sri.renderInlineText(${jsString(inline)})` : jsString(inline);
    };
    // A tui text is one node or an array of nodes. Text that is not JSON, or JSON of another shape, becomes an
    // error node naming the screen and the text, and the rest of the page is kept.
    const tuiText = (node) => {
        if (!attr(node, "location") && !(node.text || "").trim()) return "";
        const where = `String((sri.getActiveScreenDef() && sri.getActiveScreenDef().location) || "") + " " + ${jsString(attr(node, "location") || "inline")}`;
        return `{
    const $rawValue = ${rawText(node)};
    const $raw = String($rawValue == null ? "" : $rawValue);
    let $parsed;
    let $problem = "";
    try { $parsed = JSON.parse($raw); } catch ($parseError) { $problem = " is not JSON: " + $parseError.message; }
    if (!$problem && $parsed && typeof $parsed === "object" && !Array.isArray($parsed)) $parsed = [$parsed];
    if (!$problem && !(Array.isArray($parsed) && $parsed.every(($item) => $item && typeof $item === "object" && !Array.isArray($item) && typeof $item.t === "string"))) {
        $problem = " is not a node or an array of nodes";
    }
    if ($problem) {
        const $text = ${where} + $problem;
        ec.logger.warn("tui text in " + $text);
        $emit({t:"error",id:$uid(${jsString(idOf(node))}),text:$text});
    } else {
        $emit({t:"fragment",id:$uid(${jsString(idOf(node))}),c:$parsed});
    }
}
`;
    };
    // HTML as blocks and numbered links (sri.htmlText); text without markup is the same blocks.
    const htmlNode = (idExpr, htmlExpr) => `$emit(Object.assign({t:"html",id:$uid(${idExpr})}, JSON.parse(sri.htmlText(String(${htmlExpr} == null ? "" : ${htmlExpr})))));\n`;
    // An `any` text is written for the browser, so it is read as HTML.
    const anyText = (node) => {
        if (!attr(node, "location") && !(node.text || "").trim()) return "";
        return `{
    const $rawValue = ${rawText(node)};
    ${htmlNode(jsString(idOf(node)), "$rawValue")}}
`;
    };
    const textCode = (node) => {
        const type = attr(node, "type");
        if (typeNames(type).includes("tui")) return tuiText(node);
        if (!type || type === "any") return anyText(node);
        return `$emit({t:"placeholder",id:$uid(${jsString(idOf(node))}),text:"text",url:${screenUrlExpr}});\n`;
    };
    const menuItems = `const $menu = sri.getMenuItems() || [];
        const $items = [];
        for (let $mi = 0; $mi < $menu.length; $mi++) {
            const $item = $menu[$mi];
            $items.push({title:$item.title || "",url:$item.url || "",active:!!$item.inPath});
        }
`;

    const pass = stmt((node, children) => children);
    // A box toolbar, a panel side, a split side: a region node in place, so the client keeps where its nodes go.
    const regionCode = (owner, kid, name, extra) => kid && kid.source
        ? `$openChildren({t:"region",id:$uid(${jsString(idOf(owner) + "." + name)}),name:${jsString(name)}${extra || ""}});\n${kid.source}$closeChildren();\n`
        : "";
    const sizeOf = (kid, name) => (kid && /^\d+$/.test(attr(kid, name)) ? `,size:${attr(kid, name)}` : "");
    const group = (type, extra) => stmt((node, children) =>
        `$openChildren({t:${jsString(type)},id:$uid(${jsString(idOf(node))})${extra ? extra(node) : ""}});\n${children}$closeChildren();\n`);

    const overrides = {
        "screen": pass,
        "widgets": pass,
        "fail-widgets": pass,
        "dynamic-form": pass,
        // The user's columns form is a nested unit, so it and the form's own code both sit in one fragment's "c".
        "form-list-user-columns": stmt((node, children) =>
            `$openRender({t:"fragment",id:$uid(${jsString((attr(node, "name") || "list") + ".columns")})});
if (!sri.renderUserColumnsForm(${jsString(attr(node, "name") || "list")})) {
    sri.write("[");
    $stack.push($comma);
    $comma = false;
    ${children}sri.write("]");
    $comma = $stack.pop();
}
$closeRender();
`),
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
        "container-box": stmt((node) => {
            const header = child(node, "box-header");
            const title = header ? attr(header, "title") : "";
            const body = node.children.filter((kid) => kid.name !== "box-toolbar").map((kid) => kid.source || "").join("");
            return `$openChildren({t:"box",id:$uid(${jsString(idOf(node))}),title:${titleOf(title)},type:${jsString(attr(node, "type"))},initial:${jsString(attr(node, "initial"))}});
${regionCode(node, child(node, "box-toolbar"), "toolbar")}${body}$closeChildren();
`;
        }),
        "container-row": group("row"),
        "row-col": group("col", (node) =>
            `,xs:${jsString(attr(node, "xs"))},sm:${jsString(attr(node, "sm"))},md:${jsString(attr(node, "md"))},lg:${jsString(attr(node, "lg"))}`),
        "container-panel": stmt((node) => {
            const regions = ["header", "left", "center", "right", "footer"].map((name) => {
                const kid = child(node, "panel-" + name);
                return regionCode(node, kid, name, name === "left" || name === "right" ? sizeOf(kid, "size") : "");
            }).join("");
            return `$openChildren({t:"panel",id:$uid(${jsString(idOf(node))})});\n${regions}$closeChildren();\n`;
        }),
        "container-dialog": stmt((node, children) => {
            const button = attr(node, "button-text") || "";
            const title = attr(node, "title") || button;
            return guarded(node, `$openChildren({t:"dialog",id:$uid(${jsString(idOf(node))}),title:${titleOf(title)},button:${titleOf(button)}});\n${children}$closeChildren();\n`);
        }),
        "container-split": stmt((node) => {
            const width = /^\d+$/.test(attr(node, "side-width")) ? `,sideWidth:${attr(node, "side-width")}` : "";
            const regions = regionCode(node, child(node, "split-side"), "side") + regionCode(node, child(node, "split-main"), "main");
            return `$openChildren({t:"split",id:$uid(${jsString(idOf(node))})${width}${attr(node, "select-parameter") ? `,selectParameter:${jsString(attr(node, "select-parameter"))}` : ""}});\n${regions}$closeChildren();\n`;
        }),
        "dashboard": group("dashboard"),
        "section": stmt((node) =>
            `$openRender({t:"section",id:$uid(${jsString(idOf(node))}),name:${jsString(attr(node, "name") || "")}});\nsri.renderSection(${jsString(attr(node, "name") || "")});\n$closeRender();\n`),
        "section-iterate": stmt((node) =>
            `$openRender({t:"section",id:$uid(${jsString(idOf(node))}),name:${jsString(attr(node, "name") || "")},iterate:true});\nsri.renderSection(${jsString(attr(node, "name") || "")});\n$closeRender();\n`),
        "section-include": stmt((node) =>
            `$openRender({t:"section",id:$uid(${jsString(idOf(node))}),name:${jsString(attr(node, "name") || "")}});\nsri.renderSectionInclude(${S.bareLit(node)});\n$closeRender();\n`),
        "subscreens-active": stmt((node) =>
            `$openRender({t:"active",id:$uid(${jsString(attr(node, "id") || "subscreens")})});\nsri.renderSubscreen();\n$closeRender();\n`),
        "subscreens-panel": stmt((node) => {
            const type = attr(node, "type") || "tab";
            const id = attr(node, "id") || "subscreens";
            if (type === "wizard" || type === "stack") {
                return `{
                    ${menuItems}let $step = ${titleOf(attr(node, "title"))};
                    for (let $si = 0; $si < $items.length; $si++) if ($items[$si].active && $items[$si].title) $step = $items[$si].title;
                    $emit({t:"text",id:$uid(${jsString(id + "-step")}),text:String($step || "")});
                    $openRender({t:"active",id:$uid(${jsString(id)})});
                    sri.renderSubscreen();
                    $closeRender();
                }
`;
            }
            return `{
                ${menuItems}$openRender({t:"subscreens",id:$uid(${jsString(id)}),kind:${jsString(type)},items:$items});
                sri.renderSubscreen();
                $closeRender();
            }
`;
        }),
        "include-screen": stmt((node) =>
            `$openRender({t:"section",id:$uid(${jsString(attr(node, "location") || "include")}),name:"include"});\nsri.renderIncludeScreen(${jsString(attr(node, "location"))}, ${jsString(attr(node, "share-scope"))});\n$closeRender();\n`),
        "render-dynamic-form": stmt((node) => {
            const name = attr(node, "name") || "form";
            return `$openRender({t:"fragment",id:$uid(${jsString(name)})});\nsri.renderDynamicForm(${jsString(name)});\n$closeRender();\n`;
        }),
        "render-mode": stmt((node) => {
            const texts = childrenNamed(node, "text");
            const tui = texts.find((kid) => typeNames(attr(kid, "type")).includes("tui"));
            if (tui) return textCode(tui);
            const any = texts.find((kid) => !attr(kid, "type") || attr(kid, "type") === "any");
            if (any) return textCode(any);
            return `$emit({t:"placeholder",id:$uid(${jsString(idOf(node))}),text:"render-mode",url:${screenUrlExpr}});\n`;
        }),
        "text": stmt((node) => textCode(node)),
        "label": stmt((node) => guarded(node,
            `$emit({t:"text",id:$uid(${jsString(idOf(node))}),text:${S.expandExpr(attr(node, "text") || "")}${attr(node, "type") && attr(node, "type") !== "span" ? `,style:${jsString(attr(node, "type"))}` : ""}});\n`)),
        "link": stmt((node) => {
            if (fieldNodeOf(node) && formNodeOf(node)) return "";
            return linkStatement(node, inHeader(node) ? false : formNodeOf(node) && formNodeOf(node).name === "form-list");
        }),
        "form-single": formSingle,
        "form-list": formList,
        "number-card": stmt((node) => guarded(node,
            `$emit({t:"numberCard",id:$uid(${jsString(idOf(node))}),title:String(${S.expandExpr(attr(node, "title") || "")} || ""),value:String(${S.expandNoL10nExpr(attr(node, "value") || "")} || ""),url:String(${S.expandNoL10nExpr(attr(node, "url") || "")} || "")});\n`)),
        "dynamic-dialog": stmt((node) => guarded(node, `{
    const $made = sri.makeUrlByType(${jsString(attr(node, "transition") || "")}, "transition", ${S.nodeLit(node, false)}, "true");
    $emit({t:"dynamicDialog",id:$uid(${jsString(idOf(node))}),url:$made ? ($made.getUrlWithParams() || "") : "",button:String(${S.expandExpr(attr(node, "button-text") || "")} || "")});
}
`)),
        "dynamic-container": stmt((node) => `{
    const $made = sri.makeUrlByType(${jsString(attr(node, "transition") || "")}, "transition", ${S.nodeLit(node, false)}, "true");
    $emit({t:"dynamicContainer",id:$uid(${jsString(idOf(node))}),url:$made ? ($made.addParameter("_dynamic_container_id", ${jsString(idOf(node))}).getUrlWithParams() || "") : ""});
}
`),
        "image": stmt((node) =>
            `$emit({t:"image",id:$uid(${jsString(idOf(node))}),alt:String(${S.expandExpr(attr(node, "alt") || "")} || ""),url:String(${S.expandNoL10nExpr(attr(node, "url") || "")} || "")});\n`),
        "markdown-view": stmt((node) => guarded(node,
            `$emit({t:"markdown",id:$uid(${jsString(idOf(node))}),text:String(${S.markdownTextExpr(node)} || "")});\n`)),
        "status-bar": stmt((node) => guarded(node,
            `$emit({t:"statusBar",id:$uid(${jsString(idOf(node))}),text:String(${S.statusTextExpr(node)} || "")});\n`)),
        // The text when there is one, else the HTML, as the text modes read an html-view.
        "html-view": stmt((node) => guarded(node, `{
    const $text = ec.resource.expandNoL10n(${jsString(attr(node, "text") || "")});
    if ($text && $text !== "null") $emit({t:"text",id:$uid(${jsString(idOf(node))}),text:String($text).trim()});
    else {
        const $html = ec.resource.expandNoL10n(${jsString(attr(node, "html") || "")});
        ${htmlNode(jsString(idOf(node)), "($html === \"null\" ? \"\" : $html)")}    }
}
`)),
        "button-menu": stmt((node) =>
            `$emit({t:"buttonMenu",id:$uid(${jsString(idOf(node))}),text:String(${S.expandExpr(attr(node, "text") || "")} || "")});\n`),
        "tree": stmt((node) => `$emit({t:"tree",id:$uid(${jsString(attr(node, "id") || attr(node, "name") || "tree")})});\n`),
        "include-content": stmt((node) =>
            `$emit({t:"placeholder",id:$uid(${jsString(idOf(node))}),text:"include-content",url:String(${S.expandNoL10nExpr(attr(node, "location") || "")} || "")});\n`)
    };

    // Elements a form, link or list macro reads for itself, and elements the terminal does not draw (context).
    const formInternals = [
        "check", "date-find", "date-period", "date-time", "display", "display-entity", "drop-down", "dynamic-options",
        "editable", "file", "hidden", "ignored", "lookup", "password", "radio", "range-find", "reset", "submit",
        "text-area", "text-find", "text-line", "entity-options", "list-options", "option", "depends-on", "field-layout",
        "field-ref", "form-list-column", "columns", "column", "auto-fields-entity", "auto-fields-service",
        "auto-widget-entity", "auto-widget-service", "exclude", "entity-find", "entity-find-one", "set",
        "hidden-parameters", "row-selection", "row-actions", "visible-when", "editable-load", "action", "first-row",
        "second-row", "last-row", "parameter", "dialog", "tree-node", "tree-sub-node", "view-board", "view-calendar",
        "context"
    ];
    // Drawn as a placeholder with the browser link until a later phase gives them a terminal view.
    const later = ["chatter", "record-files"];
    const macros = Object.assign({}, T.noops(formInternals), overrides);
    later.forEach((name) => {
        macros[name] = stmt((node) => placeholderEmit(idOf(node), node.name, screenUrlExpr));
    });
    const unnamed = S.allElements.filter((name) => !macros[name]);
    if (unnamed.length) throw new Error("ScreenTui.js names no macro for " + unnamed.join(", "));

    return { macros, opaque: S.opaqueElements, prelude, epilogue };
})
