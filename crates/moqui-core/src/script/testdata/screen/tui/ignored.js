$emit({t:"buttonMenu",id:$uid("widgets/button-menu"),text:String(ec.resource.expand("Menu") || "")});
{
    const $made = sri.makeUrlByType("load", "transition", {"name":"dynamic-container","attrs":{"id":"Dyn","transition":"load"},"parent":{"name":"widgets","attrs":{}}}, "true");
    $emit({t:"dynamicContainer",id:$uid("Dyn"),url:$made ? ($made.addParameter("_dynamic_container_id", "Dyn").getUrlWithParams() || "") : ""});
}
{
    const $made = sri.makeUrlByType("load", "transition", {"name":"dynamic-dialog","attrs":{"id":"DynD","button-text":"Dyn","transition":"load"},"parent":{"name":"widgets","attrs":{}}}, "true");
    $emit({t:"dynamicDialog",id:$uid("DynD"),url:$made ? ($made.getUrlWithParams() || "") : "",button:String(ec.resource.expand("Dyn") || "")});
}
$emit({t:"placeholder",id:$uid("widgets/include-content"),text:"include-content",url:String(ec.resource.expandNoL10n("component://x/y.html") || "")});
$emit({t:"tree",id:$uid("T")});
