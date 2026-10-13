if (sri.doBoundaryComments()) sri.write("<!-- BEGIN screen[@location=" + sri.getActiveScreenDef().location + "].widgets -->\n");
$h.label({"name":"label","attrs":{"text":"root ${name}"}});
sri.renderSubscreen();
if (sri.doBoundaryComments()) sri.write("<!-- END   screen[@location=" + sri.getActiveScreenDef().location + "].widgets -->\n");
