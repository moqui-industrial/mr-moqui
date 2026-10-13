var total = price * 2;
var label = "Total: " + total;
$o.push($s(label));
$o.push(" (");
$o.push($s((total > 10) ? "big" : "small"));
$o.push(")\n");
$o.push("Escaped ${literal} and <#if not a tag>\n");
