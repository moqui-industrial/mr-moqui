if (sri.doBoundaryComments()) sri.write("<!-- BEGIN screen[@location=" + sri.getActiveScreenDef().location + "].widgets -->\n");
if (sri.doBoundaryComments()) sri.write("<!-- BEGIN form-single[@name=LinkForm] -->\n");
sri.pushSingleFormMapContext(fieldValues);
{
    const $form = $v.formOpen({"name":"form-single","attrs":{"name":"LinkForm"}}, [{"name": "a", "field": {"name":"field","attrs":{"name":"a"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"link","attrs":{"url":"go","text":"Go ${a}","link-type":"anchor"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"a"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}}}]}]},
{"name": "b", "field": {"name":"field","attrs":{"name":"b"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"link","attrs":{"url":"go","text":"Hidden ${b}","link-type":"hidden-form-link","condition":"b != null"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"b"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}}}]}]},
{"name": "c", "field": {"name":"field","attrs":{"name":"c"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"link","attrs":{"url":"go","text":"Plain","link-type":"auto"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"c"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}}}]}]},
{"name": "d", "field": {"name":"field","attrs":{"name":"d"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"link","attrs":{"url":"go","entity-name":"moqui.basic.Enumeration","link-type":"anchor","text":"${description}"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"d"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}}}]}]},
{"name": "e", "field": {"name":"field","attrs":{"name":"e"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"link","attrs":{"url":"go","text":"Mapped ${v}","text-map":"{v: 1}","link-type":"anchor"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"e"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}}}]}]}]);
    $v.fieldBox(null, "", false);
    sri.pushContext();
    $v.link({"name":"link","attrs":{"url":"go","text":"Go ${a}","link-type":"anchor"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"a"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}}}, true);
    sri.popContext();
    sri.write("</div>");
    $v.fieldBox(null, "", false);
    sri.pushContext();
    $v.link({"name":"link","attrs":{"url":"go","text":"Hidden ${b}","link-type":"hidden-form-link","condition":"b != null"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"b"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}}}, true);
    sri.popContext();
    sri.write("</div>");
    $v.fieldBox(null, "", false);
    sri.pushContext();
    $v.link({"name":"link","attrs":{"url":"go","text":"Plain","link-type":"auto"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"c"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}}}, true);
    sri.popContext();
    sri.write("</div>");
    $v.fieldBox(null, "", false);
    sri.pushContext();
    $v.link({"name":"link","attrs":{"url":"go","entity-name":"moqui.basic.Enumeration","link-type":"anchor","text":"${description}"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"d"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}}}, true);
    sri.popContext();
    sri.write("</div>");
    $v.fieldBox(null, "", false);
    sri.pushContext();
    $v.link({"name":"link","attrs":{"url":"go","text":"Mapped ${v}","text-map":"{v: 1}","link-type":"anchor"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"e"},"parent":{"name":"form-single","attrs":{"name":"LinkForm"}}}}}, true);
    sri.popContext();
    sri.write("</div>");
    $v.formClose({"name":"form-single","attrs":{"name":"LinkForm"}}, $form);
}
sri.popContext();
if (sri.doBoundaryComments()) sri.write("<!-- END   form-single[@name=LinkForm] -->\n");
$v.link({"name":"link","attrs":{"url":"go","text":"Standalone"}}, false);
$v.label({"name":"label","attrs":{"text":"Cond ${name}","condition":"name"}});
$v.label({"name":"label","attrs":{"text":"Mapped ${v}","text-map":"{v: 2}"}});
$v.image({"name":"image","attrs":{"url":"/img.png","alt":"An image"}});
if (sri.doBoundaryComments()) sri.write("<!-- END   screen[@location=" + sri.getActiveScreenDef().location + "].widgets -->\n");
