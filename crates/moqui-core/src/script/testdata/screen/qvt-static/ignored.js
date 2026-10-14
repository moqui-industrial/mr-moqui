{
const $bm = $s.buttonMenuOpen({"name":"button-menu","attrs":{"text":"Menu"}}, null);
if ($bm) {
sri.write($bm);
sri.write($s.buttonMenuItem({"name":"menu-item","attrs":{"text":"x"}}));
sri.write("</q-item-section></q-item>");
sri.write($s.buttonMenuClose({"name":"button-menu","attrs":{"text":"Menu"}}));
}
}
$s.dynamicContainer({"name":"dynamic-container","attrs":{"id":"Dyn","transition":"load"}});
$s.dynamicDialog({"name":"dynamic-dialog","attrs":{"id":"DynD","button-text":"Dyn","transition":"load"}});
$s.tree({"name":"tree","attrs":{"name":"T","transition":"t","root-node-name":"n"}});
