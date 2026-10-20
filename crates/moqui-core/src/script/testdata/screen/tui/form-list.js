{
    const $info = sri.openFormList("Rows");
    try {
        const $columns = [];
        $columns.push({title:[String(ec.l10n.localize("ID") || "")].filter(($part) => $part).join(" / "),orderBy:"id"});
$columns.push({title:[String(ec.l10n.localize("The Title") || "")].filter(($part) => $part).join(" / ")});
$columns.push({title:[String(ec.l10n.localize("Qty") || "")].filter(($part) => $part).join(" / ")});
$columns.push({title:[String(ec.l10n.localize("Amt") || "")].filter(($part) => $part).join(" / ")});
$columns.push({title:[String(ec.l10n.localize("Status") || "")].filter(($part) => $part).join(" / ")});
$columns.push({title:[String(ec.l10n.localize("Open") || "")].filter(($part) => $part).join(" / ")});
$columns.push({title:[String(ec.l10n.localize("Do") || "")].filter(($part) => $part).join(" / ")});
const $headerFields = [];
        $headerFields.push((() => {
            let $required = false;
            const $max = 0;
            const $hint = sri.getFieldValidation({"name":"header-field","attrs":{"show-order-by":"true"},"parent":{"name":"field","attrs":{"name":"id"},"parent":{"name":"form-list","attrs":{"name":"Rows","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}});
            if ($hint && String($hint.classes || "").split(/\s+/).indexOf("required") >= 0) $required = true;
            const $rules = sri.getFieldValidationJsRules({"name":"header-field","attrs":{"show-order-by":"true"},"parent":{"name":"field","attrs":{"name":"id"},"parent":{"name":"form-list","attrs":{"name":"Rows","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}}) || [];
            for (let $ri = 0; $ri < $rules.length; $ri++) if ($rules[$ri] && $rules[$ri].expr === "!!value") $required = true;
            const $field = {t:"field",id:"Rows.id",name:"id",title:String(ec.l10n.localize("ID") || ""),w:{kind:"textFind",value:String(sri.getFieldValueString({"name":"text-find","attrs":{},"parent":{"name":"header-field","attrs":{"show-order-by":"true"},"parent":{"name":"field","attrs":{"name":"id"},"parent":{"name":"form-list","attrs":{"name":"Rows","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}}}) || "" || "")}};
            if ($required) $field.required = true;
            if ($max > 0) $field.maxlength = $max;
            return $field;
        })());
const $rows = [];
        for (let $index = 0; $index < $info.count; $index++) {
            sri.startFormListRow($info, $index);
            try {
                const $rowKey = String($index);
                const $cells = [];
                let $cell = "";
let $piece = "";
$cell = "";
$piece = "";
$piece = String(sri.getFieldValueString({"name":"display","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"id"},"parent":{"name":"form-list","attrs":{"name":"Rows","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}}}) || "");
if ($piece && typeof $piece === "object") $cell = $piece;
else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
$cells.push($cell);
$cell = "";
$piece = "";
$piece = String(sri.getFieldValueString({"name":"text-line","attrs":{},"parent":{"name":"default-field","attrs":{"title":"The Title"},"parent":{"name":"field","attrs":{"name":"title","print-width":"30","print-width-type":"characters"},"parent":{"name":"form-list","attrs":{"name":"Rows","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}}}) || "" || "");
if ($piece && typeof $piece === "object") $cell = $piece;
else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
$cells.push($cell);
$cell = "";
$piece = "";
$piece = String(sri.getFieldValueString({"name":"display","attrs":{"format":"0.0"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"qty","align":"right","print-width":"20"},"parent":{"name":"form-list","attrs":{"name":"Rows","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}}}) || "");
if ($piece && typeof $piece === "object") $cell = $piece;
else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
$cells.push($cell);
$cell = "";
$piece = "";
if (!(hideAmt)) {
    $piece = String(ec.l10n.formatCurrency(sri.getFieldValue({"name":"field","attrs":{"name":"amt","hide":"hideAmt"},"parent":{"name":"form-list","attrs":{"name":"Rows","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}, ""), 'USD') || "");
    if ($piece && typeof $piece === "object") $cell = $piece;
    else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
}
$cells.push($cell);
$cell = "";
$piece = "";
if (status == 'X') {
    $piece = String(ec.resource.expand("Excluded") || "");
    if ($piece && typeof $piece === "object") $cell = $piece;
    else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
} else {
    $piece = String(sri.getFieldValueString({"name":"display","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"status"},"parent":{"name":"form-list","attrs":{"name":"Rows","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}}}) || "");
    if ($piece && typeof $piece === "object") $cell = $piece;
    else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
}
$cells.push($cell);
$cell = "";
$piece = "";
$piece = (() => {
            let $text = ec.resource.expand("Open ${id}");
            if ($text == null || $text === "null") $text = "";
            const $made = sri.makeUrlByType("open", "transition", {"name":"link","attrs":{"url":"open","text":"Open ${id}","link-type":"anchor"},"parent":{"name":"default-field","attrs":{"title":"Open"},"parent":{"name":"field","attrs":{"name":"link"},"parent":{"name":"form-list","attrs":{"name":"Rows","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}}}, "true");
            const $link = {t:"link",id:"Rows.link." + $rowKey,text:String($text),url:$made ? ($made.url || "") : ""};
            $link.kind = "anchor";
            
            
            return $link;
        })();
if ($piece && typeof $piece === "object") $cell = $piece;
else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
$cells.push($cell);
$cell = "";
$piece = "";
$piece = String(ec.l10n.localize("Do") || "");
if ($piece && typeof $piece === "object") $cell = $piece;
else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
$cells.push($cell);
$rows.push({cells:$cells});
            } finally {
                sri.endFormListRow();
            }
        }
        const $indexValue = "rowListPageIndex" ? ec.context.get("rowListPageIndex") : null;
        const $pageIndex = $indexValue == null || $indexValue === "" ? $whole(ec.context.get("pageIndex")) : $whole($indexValue);
        const $sizeValue = "rowListPageSize" ? ec.context.get("rowListPageSize") : null;
        const $pageSize = $sizeValue == null || $sizeValue === "" ? $whole(ec.context.get("pageSize")) : $whole($sizeValue);
        const $total = "rowListCount" ? ec.context.get("rowListCount") : null;
        const $count = $total == null || $total === "" ? $info.count : $whole($total);
        const $size = $pageSize > 0 ? $pageSize : $info.count;
        $emit({t:"formList",id:"Rows",name:"Rows",columns:$columns,rows:$rows,paging:{index:$pageIndex,size:$size,count:$count},headerFields:$headerFields});
    } finally {
        sri.safeCloseList($info);
    }
}
{
    const $info = sri.openFormList("Cols");
    try {
        const $columns = [];
        $columns.push({title:[String(ec.l10n.localize("A") || ""),String(ec.l10n.localize("B") || "")].filter(($part) => $part).join(" / ")});
$columns.push({title:[String(ec.l10n.localize("C") || "")].filter(($part) => $part).join(" / ")});
const $headerFields = [];
        const $rows = [];
        for (let $index = 0; $index < $info.count; $index++) {
            sri.startFormListRow($info, $index);
            try {
                const $rowKey = String($index);
                const $cells = [];
                let $cell = "";
let $piece = "";
$cell = "";
$piece = "";
$piece = String(sri.getFieldValueString({"name":"display","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"a"},"parent":{"name":"form-list","attrs":{"name":"Cols","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}}}) || "");
if ($piece && typeof $piece === "object") $cell = $piece;
else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
$piece = String(sri.getFieldValueString({"name":"display","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"b"},"parent":{"name":"form-list","attrs":{"name":"Cols","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}}}) || "");
if ($piece && typeof $piece === "object") $cell = $piece;
else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
$cells.push($cell);
$cell = "";
$piece = "";
$piece = String(sri.getFieldValueString({"name":"display","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"c"},"parent":{"name":"form-list","attrs":{"name":"Cols","list":"rowList"},"parent":{"name":"widgets","attrs":{}}}}}}) || "");
if ($piece && typeof $piece === "object") $cell = $piece;
else if ($piece != null && String($piece) !== "" && typeof $cell !== "object") $cell = $cell ? ($cell + " " + $piece) : String($piece);
$cells.push($cell);
$rows.push({cells:$cells});
            } finally {
                sri.endFormListRow();
            }
        }
        const $indexValue = "rowListPageIndex" ? ec.context.get("rowListPageIndex") : null;
        const $pageIndex = $indexValue == null || $indexValue === "" ? $whole(ec.context.get("pageIndex")) : $whole($indexValue);
        const $sizeValue = "rowListPageSize" ? ec.context.get("rowListPageSize") : null;
        const $pageSize = $sizeValue == null || $sizeValue === "" ? $whole(ec.context.get("pageSize")) : $whole($sizeValue);
        const $total = "rowListCount" ? ec.context.get("rowListCount") : null;
        const $count = $total == null || $total === "" ? $info.count : $whole($total);
        const $size = $pageSize > 0 ? $pageSize : $info.count;
        $emit({t:"formList",id:"Cols",name:"Cols",columns:$columns,rows:$rows,paging:{index:$pageIndex,size:$size,count:$count},headerFields:$headerFields});
    } finally {
        sri.safeCloseList($info);
    }
}
