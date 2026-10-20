{
    const $rawValue = sri.renderInlineText("this is not json");
    const $raw = String($rawValue == null ? "" : $rawValue);
    let $parsed = null;
    try { $parsed = JSON.parse($raw); } catch ($parseError) { $parsed = null; }
    if (Array.isArray($parsed)) $emit({t:"fragment",id:"widgets/text",c:$parsed});
    else $emit({t:"error",id:"widgets/text",text:String((sri.getActiveScreenDef() && sri.getActiveScreenDef().location) || "") + " " + "inline" + " is not JSON"});
}
$emit({t:"placeholder",id:"widgets/render-mode",text:"render-mode",url:(() => { const $info = sri.getScreenUrlInfo(); const $names = $info && $info.fullPathNameList; if ($names && $names.length) return "/" + $names.join("/"); const $screen = sri.getActiveScreenDef(); return ($screen && $screen.location) || ""; })()});
