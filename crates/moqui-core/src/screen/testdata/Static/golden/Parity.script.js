(function(){var m=globalThis.moqui=globalThis.moqui||{};(m.qvtStatic=m.qvtStatic||{})["/Parity"]=function(Vue){
const { toDisplayString: _toDisplayString, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, Fragment: _Fragment } = Vue

return function render(_ctx, _cache) {
  return (_openBlock(), _createElementBlock(_Fragment, null, [
    (((_ctx.$mq.compare(_ctx.status,'equals','ACTIVE',null,null,'Object') && (_ctx.qty > 3))))
      ? (_openBlock(), _createElementBlock("p", {
          key: 0,
          class: "text-inline",
          textContent: _toDisplayString(_ctx.ec.l10n.localize('the condition holds'))
        }, null, 8, ["textContent"]))
      : (_openBlock(), _createElementBlock("p", {
          key: 1,
          class: "text-inline",
          textContent: _toDisplayString(_ctx.ec.l10n.localize('the condition fails'))
        }, null, 8, ["textContent"])),
    (((_ctx.$mq.compare(_ctx.qty,'greater','10',null,null,'Long') || !((_ctx.status)))))
      ? (_openBlock(), _createElementBlock("p", {
          key: 2,
          class: "text-inline",
          textContent: _toDisplayString(_ctx.ec.l10n.localize('the second holds'))
        }, null, 8, ["textContent"]))
      : (_openBlock(), _createElementBlock("p", {
          key: 3,
          class: "text-inline",
          textContent: _toDisplayString(_ctx.ec.l10n.localize('the second fails'))
        }, null, 8, ["textContent"]))
  ], 64))
}
};})();
