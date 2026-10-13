if (sri.doBoundaryComments()) sri.write("<!-- BEGIN screen[@location=" + sri.getActiveScreenDef().location + "].fail-widgets -->\n");
$h.label({"name":"label","attrs":{"text":"failed ${name}"}});
if (sri.doBoundaryComments()) sri.write("<!-- END   screen[@location=" + sri.getActiveScreenDef().location + "].fail-widgets -->\n");
