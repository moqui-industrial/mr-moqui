$o.push("Start\n");
if (status == "open") {
$o.push("Open ");
$o.push($s(count));
$o.push("\n");
} else if (status == "closed") {
$o.push("Closed\n");
} else {
$o.push("Other\n");
}
if (!items.length) {
$o.push("No items");
} else {
$o.push($s(items.length));
$o.push(" items");
}
$o.push("\n");
$o.push("End\n");
