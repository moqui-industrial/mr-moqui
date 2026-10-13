if (sri.doBoundaryComments()) sri.write("<!-- BEGIN screen[@location=" + sri.getActiveScreenDef().location + "].widgets -->\n");
$v.label({"name":"label","attrs":{"text":"root ${name}"}});
sri.write("<m-subscreens-active></m-subscreens-active>");
if (sri.doBoundaryComments()) sri.write("<!-- END   screen[@location=" + sri.getActiveScreenDef().location + "].widgets -->\n");
