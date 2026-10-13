{
    const $label = ec.resource.expand("Title ${name}");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
sri.write("<div id=\"Box1\">");
{
    const $label = ec.resource.expand("inside");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
{
    const $label = ec.resource.expand("left");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
{
    const $label = ec.resource.expand("right");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
sri.write("</div>\n");
sri.write('<div class="box-header">');
{
    const $label = ec.resource.expand("Head");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
sri.write("</div>\n");
{
    const $label = ec.resource.expand("Body");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
{
    const $label = ec.resource.expand("Nopad");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
sri.write("<div>");
{
    const $label = ec.resource.expand("ph");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
sri.write("</div>\n");
sri.write('<table class="panel"><tr>');
sri.write("<td>");
{
    const $label = ec.resource.expand("pl");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
sri.write("</td>");
sri.write("<td>");
{
    const $label = ec.resource.expand("pc");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
sri.write("</td>");
sri.write("<td>");
{
    const $label = ec.resource.expand("pr");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
sri.write("</td>");
sri.write("</tr></table>\n");
sri.write("<div>");
{
    const $label = ec.resource.expand("pf");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
sri.write("</div>\n");
sri.renderSection("First");
sri.renderSection("Each");
sri.renderSectionInclude({"name":"section-include","attrs":{"name":"First"}});
sri.renderSubscreen();
sri.renderSubscreen();
sri.renderIncludeScreen("component://x/screen/Other.xml", "true");
