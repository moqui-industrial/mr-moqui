$emit({t:"buttonMenu",id:"widgets/button-menu",text:String(ec.resource.expand("Menu") || "")});
{
    const $made = sri.makeUrlByType("load", "transition", {"name":"dynamic-container","attrs":{"id":"Dyn","transition":"load"},"parent":{"name":"widgets","attrs":{}}}, "true");
    $emit({t:"dynamicContainer",id:"Dyn",url:$made ? ($made.url || "") : ""});
}
{
    const $made = sri.makeUrlByType("load", "transition", {"name":"dynamic-dialog","attrs":{"id":"DynD","button-text":"Dyn","transition":"load"},"parent":{"name":"widgets","attrs":{}}}, "true");
    $emit({t:"dynamicDialog",id:"DynD",url:$made ? ($made.url || "") : "",button:String(ec.resource.expand("Dyn") || "")});
}
$emit({t:"placeholder",id:"widgets/include-content",text:"include-content",url:String(ec.resource.expandNoL10n("component://x/y.html") || "")});
$emit({t:"tree",id:"T"});
