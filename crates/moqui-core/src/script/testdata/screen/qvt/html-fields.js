if (sri.doBoundaryComments()) sri.write("<!-- BEGIN screen[@location=" + sri.getActiveScreenDef().location + "].widgets -->\n");
if (sri.doBoundaryComments()) sri.write("<!-- BEGIN form-single[@name=Everything] -->\n");
sri.pushSingleFormMapContext(fv);
try {
    {
        const $form = $v.formOpen({"name":"form-single","attrs":{"name":"Everything","map":"fv","background-submit":"true","background-reload-id":"Dyn","focus-field":"firstName","transition":"save","pass-through-parameters":"true"}}, [{"name": "firstName", "field": {"name":"field","attrs":{"name":"firstName"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"text-line","attrs":{"size":"20","maxlength":"40","mask":"999"},"parent":{"name":"default-field","attrs":{"title":"First"},"parent":{"name":"field","attrs":{"name":"firstName"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "email", "field": {"name":"field","attrs":{"name":"email"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"text-line","attrs":{"input-type":"email","validate-service":"x.y.Z","validate-parameter":"mail","disabled":"false"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"email"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "ac", "field": {"name":"field","attrs":{"name":"ac"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"text-line","attrs":{"ac-transition":"find","ac-min-length":"2","ac-show-value":"true","ac-use-actual":"true"},"children":[{"name":"depends-on","attrs":{"field":"firstName","parameter":"fn"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"ac"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "dflt", "field": {"name":"field","attrs":{"name":"dflt"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"text-line","attrs":{"default-transition":"fill"},"children":[{"name":"depends-on","attrs":{"field":"firstName"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"dflt"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "bio", "field": {"name":"field","attrs":{"name":"bio"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"text-area","attrs":{"rows":"4","editor-type":"html"},"parent":{"name":"default-field","attrs":{"tooltip":"About you"},"parent":{"name":"field","attrs":{"name":"bio"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "find", "field": {"name":"field","attrs":{"name":"find"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"text-find","attrs":{"default-operator":"begins"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"find"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "status", "field": {"name":"field","attrs":{"name":"status"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"drop-down","attrs":{"allow-empty":"true","show-not":"true","current-description":"Gone"},"children":[{"name":"option","attrs":{"key":"A","text":"Active"}},{"name":"option","attrs":{"key":"${k}"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"status"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "multi", "field": {"name":"field","attrs":{"name":"multi"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"drop-down","attrs":{"allow-multiple":"true","combo-box":"true"},"children":[{"name":"option","attrs":{"key":"A"}},{"name":"option","attrs":{"key":"B"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"multi"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "remote", "field": {"name":"field","attrs":{"name":"remote"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"drop-down","attrs":{"allow-empty":"true"},"children":[{"name":"dynamic-options","attrs":{"transition":"options","server-search":"true","min-length":"2","label-field":"text","value-field":"id"},"children":[{"name":"depends-on","attrs":{"field":"status","parameter":"st"}}]}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"remote"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "kind", "field": {"name":"field","attrs":{"name":"kind"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"radio","attrs":{"no-current-selected-key":"X"},"children":[{"name":"option","attrs":{"key":"X"}},{"name":"option","attrs":{"key":"Y","text":"Why"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"kind"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "flag", "field": {"name":"field","attrs":{"name":"flag"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"check","attrs":{"all-checked":"false"},"children":[{"name":"option","attrs":{"key":"Y","text":"Yes"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"flag"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "stamp", "field": {"name":"field","attrs":{"name":"stamp"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"date-time","attrs":{"minute-stepping":"10"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"stamp"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "day", "field": {"name":"field","attrs":{"name":"day"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"date-time","attrs":{"type":"date","format":"MM/dd/yyyy"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"day"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "at", "field": {"name":"field","attrs":{"name":"at"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"date-time","attrs":{"type":"time","default-value":"12:00"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"at"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "between", "field": {"name":"field","attrs":{"name":"between"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"date-find","attrs":{"type":"date"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"between"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "period", "field": {"name":"field","attrs":{"name":"period"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"date-period","attrs":{"allow-empty":"false"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"period"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "range", "field": {"name":"field","attrs":{"name":"range"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"range-find","attrs":{"size":"6"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"range"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "amount", "field": {"name":"field","attrs":{"name":"amount"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"display","attrs":{"currency-unit-field":"uom","style":"big"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"amount"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "shown", "field": {"name":"field","attrs":{"name":"shown"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"display","attrs":{"text":"Total ${total}","also-hidden":"false"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"shown"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "dynamicShown", "field": {"name":"field","attrs":{"name":"dynamicShown"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"display","attrs":{"dynamic-transition":"lookup"},"children":[{"name":"depends-on","attrs":{"field":"status","parameter":"st"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"dynamicShown"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "enumId", "field": {"name":"field","attrs":{"name":"enumId"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"display-entity","attrs":{"entity-name":"moqui.basic.Enumeration","text":"${description}"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"enumId"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "doc", "field": {"name":"field","attrs":{"name":"doc"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"file","attrs":{"accept":"image/*","multiple":"true"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"doc"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "pw", "field": {"name":"field","attrs":{"name":"pw"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"password","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"pw"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "where", "field": {"name":"field","attrs":{"name":"where"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"lookup","attrs":{"size":"12"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"where"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "clear", "field": {"name":"field","attrs":{"name":"clear"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"reset","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"clear"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "secret", "field": {"name":"field","attrs":{"name":"secret"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"hidden","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"secret"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "skip", "field": {"name":"field","attrs":{"name":"skip"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"ignored","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"skip"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "hid", "field": {"name":"field","attrs":{"name":"hid","hide":"true"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"display","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"hid","hide":"true"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "mix", "field": {"name":"field","attrs":{"name":"mix"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": "mode == 'a'", "sets": [], "widgets": [{"name":"text-line","attrs":{},"parent":{"name":"conditional-field","attrs":{"condition":"mode == 'a'"},"parent":{"name":"field","attrs":{"name":"mix"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]},{"cond": null, "sets": [], "widgets": [{"name":"display","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"mix"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "go", "field": {"name":"field","attrs":{"name":"go"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"submit","attrs":{"confirmation":"Sure?","icon":"fa fa-check"},"parent":{"name":"default-field","attrs":{"title":"Go Now"},"parent":{"name":"field","attrs":{"name":"go"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]},
{"name": "viaLink", "field": {"name":"field","attrs":{"name":"viaLink"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}, "subs": [{"cond": null, "sets": [], "widgets": [{"name":"link","attrs":{"url":"open","text":"Open","link-type":"auto"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"viaLink"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}]}]}]);
        $v.fieldGroup({"name":"field-group","attrs":{"title":"Names","box":"true"}});
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.textLine({"name":"text-line","attrs":{"size":"20","maxlength":"40","mask":"999"},"parent":{"name":"default-field","attrs":{"title":"First"},"parent":{"name":"field","attrs":{"name":"firstName"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.textLine({"name":"text-line","attrs":{"input-type":"email","validate-service":"x.y.Z","validate-parameter":"mail","disabled":"false"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"email"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldGroupClose({"name":"field-group","attrs":{"title":"Names","box":"true"}});
        $v.accordion("Acc", "2");
        sri.write("<q-list bordered class=\"rounded-borders q-my-sm\">");
        $v.fieldGroup({"name":"field-group","attrs":{"title":"One"}});
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.textLine({"name":"text-line","attrs":{"ac-transition":"find","ac-min-length":"2","ac-show-value":"true","ac-use-actual":"true"},"children":[{"name":"depends-on","attrs":{"field":"firstName","parameter":"fn"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"ac"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldGroupClose({"name":"field-group","attrs":{"title":"One"}});
        $v.fieldGroup({"name":"field-group","attrs":{"title":"Two"}});
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.textLine({"name":"text-line","attrs":{"default-transition":"fill"},"children":[{"name":"depends-on","attrs":{"field":"firstName"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"dflt"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldGroupClose({"name":"field-group","attrs":{"title":"Two"}});
        sri.write("</q-list>\n");
        $v.endAccordion();
        sri.write("<div class=\"row\">");
        sri.write("<div class=\"col-xs-12 col-sm-6\">");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.textArea({"name":"text-area","attrs":{"rows":"4","editor-type":"html"},"parent":{"name":"default-field","attrs":{"tooltip":"About you"},"parent":{"name":"field","attrs":{"name":"bio"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        sri.write("</div>");
        sri.write("<div class=\"col-xs-12 col-sm-6\">");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.textFind({"name":"text-find","attrs":{"default-operator":"begins"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"find"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        sri.write("</div>");
        sri.write("</div>\n");
        sri.write("<div class=\"q-my-sm big-row\"" + "" + ">");
        sri.write("<div class=\"big-row-item\">" + $v.t(ec.resource.expand("Big", "")) + "</div>");
        $v.fieldBox(null, "", true);
        sri.pushContext();
        try {
            $v.dropDown({"name":"drop-down","attrs":{"allow-empty":"true","show-not":"true","current-description":"Gone"},"children":[{"name":"option","attrs":{"key":"A","text":"Active"}},{"name":"option","attrs":{"key":"${k}"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"status"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", true);
        sri.pushContext();
        try {
            $v.dropDown({"name":"drop-down","attrs":{"allow-multiple":"true","combo-box":"true"},"children":[{"name":"option","attrs":{"key":"A"}},{"name":"option","attrs":{"key":"B"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"multi"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        sri.write("</div>\n");
        $v.rowOpen("row", "cols");
        $v.colOpen({"name":"field-col","attrs":{"md":"6","label-cols":"3"}}, true);
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.dropDown({"name":"drop-down","attrs":{"allow-empty":"true"},"children":[{"name":"dynamic-options","attrs":{"transition":"options","server-search":"true","min-length":"2","label-field":"text","value-field":"id"},"children":[{"name":"depends-on","attrs":{"field":"status","parameter":"st"}}]}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"remote"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        sri.write("</div>");
        $v.colOpen({"name":"field-col","attrs":{"md":"6"}}, true);
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.radio({"name":"radio","attrs":{"no-current-selected-key":"X"},"children":[{"name":"option","attrs":{"key":"X"}},{"name":"option","attrs":{"key":"Y","text":"Why"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"kind"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        sri.write("</div>");
        sri.write("</div>\n");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.check({"name":"check","attrs":{"all-checked":"false"},"children":[{"name":"option","attrs":{"key":"Y","text":"Yes"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"flag"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.dateTime({"name":"date-time","attrs":{"minute-stepping":"10"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"stamp"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.dateTime({"name":"date-time","attrs":{"type":"date","format":"MM/dd/yyyy"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"day"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.dateTime({"name":"date-time","attrs":{"type":"time","default-value":"12:00"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"at"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.dateFind({"name":"date-find","attrs":{"type":"date"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"between"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.datePeriod({"name":"date-period","attrs":{"allow-empty":"false"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"period"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.rangeFind({"name":"range-find","attrs":{"size":"6"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"range"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.display({"name":"display","attrs":{"currency-unit-field":"uom","style":"big"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"amount"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.display({"name":"display","attrs":{"text":"Total ${total}","also-hidden":"false"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"shown"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.display({"name":"display","attrs":{"dynamic-transition":"lookup"},"children":[{"name":"depends-on","attrs":{"field":"status","parameter":"st"}}],"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"dynamicShown"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.displayEntity({"name":"display-entity","attrs":{"entity-name":"moqui.basic.Enumeration","text":"${description}"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"enumId"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.file({"name":"file","attrs":{"accept":"image/*","multiple":"true"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"doc"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.password({"name":"password","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"pw"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.textLine({"name":"lookup","attrs":{"size":"12"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"where"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.reset_({"name":"reset","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"clear"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.hidden({"name":"hidden","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"secret"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        if (!ec.resource.condition("true", "")) {
            $v.fieldBox(null, "", false);
            sri.pushContext();
            try {
                $v.display({"name":"display","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"hid","hide":"true"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
            } finally {
                sri.popContext();
            }
            sri.write("</div>");
        }
        if (mode == 'a') {
            $v.fieldBox(null, "", false);
            sri.pushContext();
            try {
                $v.textLine({"name":"text-line","attrs":{},"parent":{"name":"conditional-field","attrs":{"condition":"mode == 'a'"},"parent":{"name":"field","attrs":{"name":"mix"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
            } finally {
                sri.popContext();
            }
            sri.write("</div>");
        } else {
            $v.fieldBox(null, "", false);
            sri.pushContext();
            try {
                $v.display({"name":"display","attrs":{},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"mix"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
            } finally {
                sri.popContext();
            }
            sri.write("</div>");
        }
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.submit({"name":"submit","attrs":{"confirmation":"Sure?","icon":"fa fa-check"},"parent":{"name":"default-field","attrs":{"title":"Go Now"},"parent":{"name":"field","attrs":{"name":"go"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}});
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.fieldBox(null, "", false);
        sri.pushContext();
        try {
            $v.link({"name":"link","attrs":{"url":"open","text":"Open","link-type":"auto"},"parent":{"name":"default-field","attrs":{},"parent":{"name":"field","attrs":{"name":"viaLink"},"parent":{"name":"form-single","attrs":{"name":"Everything","map":"fv"}}}}}, true);
        } finally {
            sri.popContext();
        }
        sri.write("</div>");
        $v.formClose({"name":"form-single","attrs":{"name":"Everything","map":"fv","background-submit":"true","background-reload-id":"Dyn","focus-field":"firstName","transition":"save","pass-through-parameters":"true"}}, $form);
    }
} finally {
    sri.popContext();
}
if (sri.doBoundaryComments()) sri.write("<!-- END   form-single[@name=Everything] -->\n");
if (sri.doBoundaryComments()) sri.write("<!-- END   screen[@location=" + sri.getActiveScreenDef().location + "].widgets -->\n");
