{
    const $rawValue = sri.renderInlineText("for any mode ${name}");
    $emit({t:"text",id:"widgets/render-mode[0]/text[1]",text:((t, h) => { const v = t && t !== "null" ? String(t) : (h && h !== "null" ? String(h).replace(/<(style|script)[^>]*>[\s\S]*?<\/\1>/gi, " ").replace(/<br\s*\/?>/gi, "\n").replace(/<\/(p|div|li|tr|h[1-6]|blockquote)>/gi, "\n").replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, "&") : ""); return v.replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim(); })(String($rawValue == null ? "" : $rawValue), "")});
}
$emit({t:"placeholder",id:"widgets/render-mode[1]",text:"render-mode",url:(() => { const $info = sri.getScreenUrlInfo(); const $names = $info && $info.fullPathNameList; if ($names && $names.length) return "/" + $names.join("/"); const $screen = sri.getActiveScreenDef(); return ($screen && $screen.location) || ""; })()});
$emit({t:"placeholder",id:"widgets/text[0]",text:"text",url:(() => { const $info = sri.getScreenUrlInfo(); const $names = $info && $info.fullPathNameList; if ($names && $names.length) return "/" + $names.join("/"); const $screen = sri.getActiveScreenDef(); return ($screen && $screen.location) || ""; })()});
$emit({t:"placeholder",id:"widgets/text[1]",text:"text",url:(() => { const $info = sri.getScreenUrlInfo(); const $names = $info && $info.fullPathNameList; if ($names && $names.length) return "/" + $names.join("/"); const $screen = sri.getActiveScreenDef(); return ($screen && $screen.location) || ""; })()});
{
    const $rawValue = sri.renderText(ec.resource.expandNoL10n("component://x/template/Part.txt"), "false");
    $emit({t:"text",id:"widgets/text[2]",text:((t, h) => { const v = t && t !== "null" ? String(t) : (h && h !== "null" ? String(h).replace(/<(style|script)[^>]*>[\s\S]*?<\/\1>/gi, " ").replace(/<br\s*\/?>/gi, "\n").replace(/<\/(p|div|li|tr|h[1-6]|blockquote)>/gi, "\n").replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, "&") : ""); return v.replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim(); })(String($rawValue == null ? "" : $rawValue), "")});
}
{
    const $rawValue = sri.renderText(ec.resource.expandNoL10n("component://x/template/${name}.txt"), "true");
    $emit({t:"text",id:"widgets/text[3]",text:((t, h) => { const v = t && t !== "null" ? String(t) : (h && h !== "null" ? String(h).replace(/<(style|script)[^>]*>[\s\S]*?<\/\1>/gi, " ").replace(/<br\s*\/?>/gi, "\n").replace(/<\/(p|div|li|tr|h[1-6]|blockquote)>/gi, "\n").replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, "&") : ""); return v.replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim(); })(String($rawValue == null ? "" : $rawValue), "")});
}
{
    const $rawValue = "a < b";
    $emit({t:"text",id:"widgets/text[4]",text:((t, h) => { const v = t && t !== "null" ? String(t) : (h && h !== "null" ? String(h).replace(/<(style|script)[^>]*>[\s\S]*?<\/\1>/gi, " ").replace(/<br\s*\/?>/gi, "\n").replace(/<\/(p|div|li|tr|h[1-6]|blockquote)>/gi, "\n").replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, "&") : ""); return v.replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim(); })(String($rawValue == null ? "" : $rawValue), "")});
}
{
    const $rawValue = sri.renderInlineText("quiet ${name}");
    $emit({t:"text",id:"widgets/text[5]",text:((t, h) => { const v = t && t !== "null" ? String(t) : (h && h !== "null" ? String(h).replace(/<(style|script)[^>]*>[\s\S]*?<\/\1>/gi, " ").replace(/<br\s*\/?>/gi, "\n").replace(/<\/(p|div|li|tr|h[1-6]|blockquote)>/gi, "\n").replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, "&") : ""); return v.replace(/[ \t]+\n/g, "\n").replace(/\n{3,}/g, "\n\n").trim(); })(String($rawValue == null ? "" : $rawValue), "")});
}
