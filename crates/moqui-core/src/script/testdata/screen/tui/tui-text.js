{
    const $rawValue = sri.renderInlineText("this is not json");
    const $raw = String($rawValue == null ? "" : $rawValue);
    let $parsed;
    let $problem = "";
    try { $parsed = JSON.parse($raw); } catch ($parseError) { $problem = " is not JSON: " + $parseError.message; }
    if (!$problem && $parsed && typeof $parsed === "object" && !Array.isArray($parsed)) $parsed = [$parsed];
    if (!$problem && !(Array.isArray($parsed) && $parsed.every(($item) => $item && typeof $item === "object" && !Array.isArray($item) && typeof $item.t === "string"))) {
        $problem = " is not a node or an array of nodes";
    }
    if ($problem) {
        const $text = String((sri.getActiveScreenDef() && sri.getActiveScreenDef().location) || "") + " " + "inline" + $problem;
        ec.logger.warn("tui text in " + $text);
        $emit({t:"error",id:$uid("widgets/text"),text:$text});
    } else {
        $emit({t:"fragment",id:$uid("widgets/text"),c:$parsed});
    }
}
$emit({t:"placeholder",id:$uid("widgets/render-mode"),text:"render-mode",url:(() => { const $info = sri.getScreenUrlInfo(); const $names = $info && $info.fullPathNameList; if ($names && $names.length) return "/" + $names.join("/"); const $screen = sri.getActiveScreenDef(); return ($screen && $screen.location) || ""; })()});
