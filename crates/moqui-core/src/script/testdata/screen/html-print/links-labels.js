sri.pushSingleFormMapContext(fieldValues);
try {
    sri.write('<table class="field"><tr><td class="field-title">' + $e(ec.l10n.localize("A")) + '</td><td class="field-value">');
    {
        let $linkText = ec.resource.expand("Go ${a}");
        if ($linkText === "null") $linkText = "";
        const $url = sri.makeUrlByType("go", "transition", {"name":"link","attrs":{"url":"go","text":"Go ${a}","link-type":"anchor"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"a"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"},"parent":{"name":"widgets","attrs":{}}}}}}, "true");
        sri.write('<a href="' + $e($url.getUrlWithParams()) + '">' + $e($linkText) + "</a>");
    }
    sri.write("</td></tr></table>\n");
    sri.write('<table class="field"><tr><td class="field-title">' + $e(ec.l10n.localize("B")) + '</td><td class="field-value">');
    if (b != null) {
        {
            let $linkText = ec.resource.expand("Hidden ${b}");
            if ($linkText === "null") $linkText = "";
            const $url = sri.makeUrlByType("go", "transition", {"name":"link","attrs":{"url":"go","text":"Hidden ${b}","link-type":"hidden-form-link","condition":"b != null"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"b"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"},"parent":{"name":"widgets","attrs":{}}}}}}, "true");
            sri.write('<a href="' + $e($url.getUrlWithParams()) + '">' + $e($linkText) + "</a>");
        }
    }
    sri.write("</td></tr></table>\n");
    sri.write('<table class="field"><tr><td class="field-title">' + $e(ec.l10n.localize("C")) + '</td><td class="field-value">');
    {
        let $linkText = ec.resource.expand("Plain");
        if ($linkText === "null") $linkText = "";
        const $url = sri.makeUrlByType("go", "transition", {"name":"link","attrs":{"url":"go","text":"Plain","link-type":"auto"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"c"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"},"parent":{"name":"widgets","attrs":{}}}}}}, "true");
        sri.write('<a href="' + $e($url.getUrlWithParams()) + '">' + $e($linkText) + "</a>");
    }
    sri.write("</td></tr></table>\n");
    sri.write('<table class="field"><tr><td class="field-title">' + $e(ec.l10n.localize("D")) + '</td><td class="field-value">');
    {
        let $linkText = sri.getFieldEntityValue({"name":"link","attrs":{"url":"go","entity-name":"moqui.basic.Enumeration","link-type":"anchor","text":"${description}"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"d"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"},"parent":{"name":"widgets","attrs":{}}}}}});
        if ($linkText === "null") $linkText = "";
        const $url = sri.makeUrlByType("go", "transition", {"name":"link","attrs":{"url":"go","entity-name":"moqui.basic.Enumeration","link-type":"anchor","text":"${description}"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"d"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"},"parent":{"name":"widgets","attrs":{}}}}}}, "true");
        sri.write('<a href="' + $e($url.getUrlWithParams()) + '">' + $e($linkText) + "</a>");
    }
    sri.write("</td></tr></table>\n");
    sri.write('<table class="field"><tr><td class="field-title">' + $e(ec.l10n.localize("E")) + '</td><td class="field-value">');
    {
        let $linkText = ec.resource.expand("Mapped ${v}", "", {v: 1});
        if ($linkText === "null") $linkText = "";
        const $url = sri.makeUrlByType("go", "transition", {"name":"link","attrs":{"url":"go","text":"Mapped ${v}","text-map":"{v: 1}","link-type":"anchor"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"e"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"},"parent":{"name":"widgets","attrs":{}}}}}}, "true");
        sri.write('<a href="' + $e($url.getUrlWithParams()) + '">' + $e($linkText) + "</a>");
    }
    sri.write("</td></tr></table>\n");
} finally {
    sri.popContext();
}
sri.write("<div>");
{
    let $linkText = ec.resource.expand("Standalone");
    if ($linkText === "null") $linkText = "";
    const $url = sri.makeUrlByType("go", "transition", {"name":"link","attrs":{"url":"go","text":"Standalone"},"parent":{"name":"widgets","attrs":{}}}, "true");
    sri.write('<a href="' + $e($url.getUrlWithParams()) + '">' + $e($linkText) + "</a>");
}
sri.write("</div>\n");
if (name) {
    {
        const $label = ec.resource.expand("Cond ${name}");
        if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
    }
}
{
    const $label = ec.resource.expand("Mapped ${v}", "", {v: 2});
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
{
    const $url = sri.makeUrlByType("/img.png", "plain", null, "true");
    sri.write('<img src="' + $e($url.url) + '" alt="' + $e("An image") + '"' + "/>\n");
}
