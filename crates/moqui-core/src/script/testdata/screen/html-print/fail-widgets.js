{
    const $label = ec.resource.expand("failed ${name}");
    if (String($label).trim() !== "") sri.write("<div>" + $e($label) + "</div>\n");
}
