(function(){var m=globalThis.moqui=globalThis.moqui||{};(m.qvtStatic=m.qvtStatic||{})["/Parity"]=function(Vue){
const { toDisplayString: _toDisplayString, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, createElementVNode: _createElementVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, createVNode: _createVNode, createBlock: _createBlock, renderList: _renderList, Fragment: _Fragment } = Vue

return function render(_ctx, _cache) {
  const _component_m_link = _resolveComponent("m-link")
  const _component_m_form_paginate = _resolveComponent("m-form-paginate")

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
        }, null, 8, ["textContent"])),
    _createElementVNode("p", {
      class: "text-inline",
      textContent: _toDisplayString(_ctx.ec.resource.expand('Product ${productName} at ${price}',{'productName':()=>(_ctx.$mq.inMap((_ctx.product),'productName')?_ctx.$mq.inMap((_ctx.product),'productName',true):(_ctx.productName)),'price':()=>(_ctx.$mq.inMap((_ctx.product),'price')?_ctx.$mq.inMap((_ctx.product),'price',true):(_ctx.price))}))
    }, null, 8, ["textContent"]),
    _createVNode(_component_m_link, {
      href: _ctx.$mq.url('/Parity/viewProduct',{},null)
    }, {
      default: _withCtx(() => [
        _createElementVNode("span", {
          textContent: _toDisplayString(_ctx.ec.resource.expand('View ${productName}',{'productName':()=>(_ctx.$mq.inMap((_ctx.product),'productName')?_ctx.$mq.inMap((_ctx.product),'productName',true):(_ctx.productName))}))
        }, null, 8, ["textContent"])
      ]),
      _: 1
    }, 8, ["href"]),
    (_ctx.$scope['linesCount'])
      ? (_openBlock(), _createBlock(_component_m_form_paginate, {
          key: 4,
          paginate: {count:Number(_ctx.$scope['linesCount']),pageIndex:Number(_ctx.$scope['linesPageIndex']),pageSize:Number(_ctx.$scope['linesPageSize']),pageMaxIndex:Number(_ctx.$scope['linesPageMaxIndex']),pageRangeLow:Number(_ctx.$scope['linesPageRangeLow']),pageRangeHigh:Number(_ctx.$scope['linesPageRangeHigh'])}
        }, null, 8, ["paginate"]))
      : _createCommentVNode("", true),
    (_openBlock(true), _createElementBlock(_Fragment, null, _renderList((_ctx.lines), (line, line_index) => {
      return (_openBlock(), _createElementBlock("div", {
        key: line_index,
        class: "text-inline",
        textContent: _toDisplayString(_ctx.ec.resource.expand('line ${line.lineId}',{'line.lineId':()=>(line.lineId)}))
      }, null, 8, ["textContent"]))
    }), 128))
  ], 64))
}
};})();
