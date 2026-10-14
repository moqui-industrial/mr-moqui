(function(){var m=globalThis.moqui=globalThis.moqui||{};(m.qvtStatic=m.qvtStatic||{})["/Parity"]=function(Vue){
const { toDisplayString: _toDisplayString, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, createElementVNode: _createElementVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, createVNode: _createVNode, createBlock: _createBlock, normalizeClass: _normalizeClass, renderList: _renderList, Fragment: _Fragment } = Vue

return function render(_ctx, _cache) {
  const _component_m_link = _resolveComponent("m-link")
  const _component_q_item_section = _resolveComponent("q-item-section")
  const _component_q_item = _resolveComponent("q-item")
  const _component_q_list = _resolveComponent("q-list")
  const _component_q_btn_dropdown = _resolveComponent("q-btn-dropdown")
  const _component_m_text_line = _resolveComponent("m-text-line")
  const _component_q_expansion_item = _resolveComponent("q-expansion-item")
  const _component_m_text_area = _resolveComponent("m-text-area")
  const _component_m_form = _resolveComponent("m-form")
  const _component_m_scope = _resolveComponent("m-scope")
  const _component_m_display = _resolveComponent("m-display")
  const _component_m_static_form_list = _resolveComponent("m-static-form-list")
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
    _createVNode(_component_q_btn_dropdown, {
      dense: "",
      outline: "",
      "no-caps": "",
      color: "primary"
    }, {
      label: _withCtx(() => [
        _createElementVNode("i", { class: "fa fa-bars q-icon on-left" }),
        _createElementVNode("span", {
          textContent: _toDisplayString(_ctx.ec.resource.expand('Menu of ${productName}',{'productName':()=>(_ctx.$mq.inMap((_ctx.product),'productName')?_ctx.$mq.inMap((_ctx.product),'productName',true):(_ctx.productName))}))
        }, null, 8, ["textContent"])
      ]),
      default: _withCtx(() => [
        _createVNode(_component_q_list, null, {
          default: _withCtx(() => [
            _createVNode(_component_q_item, null, {
              default: _withCtx(() => [
                _createVNode(_component_q_item_section, null, {
                  default: _withCtx(() => [
                    _createVNode(_component_m_link, {
                      href: _ctx.$mq.url('/Parity/viewProduct',{},null)
                    }, {
                      default: _withCtx(() => [
                        _createElementVNode("span", {
                          textContent: _toDisplayString(_ctx.ec.l10n.localize('View it'))
                        }, null, 8, ["textContent"])
                      ]),
                      _: 1
                    }, 8, ["href"])
                  ]),
                  _: 1
                })
              ]),
              _: 1
            }),
            ((_ctx.qty > 3))
              ? (_openBlock(), _createBlock(_component_q_item, { key: 0 }, {
                  default: _withCtx(() => [
                    _createVNode(_component_q_item_section, null, {
                      default: _withCtx(() => [
                        ((_ctx.qty > 3))
                          ? (_openBlock(), _createElementBlock("span", {
                              key: 0,
                              class: "text-inline",
                              textContent: _toDisplayString(_ctx.ec.l10n.localize('shown when qty is over 3'))
                            }, null, 8, ["textContent"]))
                          : _createCommentVNode("", true)
                      ]),
                      _: 1
                    })
                  ]),
                  _: 1
                }))
              : _createCommentVNode("", true),
            ((_ctx.qty <= 3))
              ? (_openBlock(), _createBlock(_component_q_item, { key: 1 }, {
                  default: _withCtx(() => [
                    _createVNode(_component_q_item_section, null, {
                      default: _withCtx(() => [
                        ((_ctx.qty <= 3))
                          ? (_openBlock(), _createElementBlock("span", {
                              key: 0,
                              class: "text-inline",
                              textContent: _toDisplayString(_ctx.ec.l10n.localize('hidden when qty is over 3'))
                            }, null, 8, ["textContent"]))
                          : _createCommentVNode("", true)
                      ]),
                      _: 1
                    })
                  ]),
                  _: 1
                }))
              : _createCommentVNode("", true)
          ]),
          _: 1
        })
      ]),
      _: 1
    }),
    _createVNode(_component_m_scope, {
      value: (_ctx.product),
      extra: _ctx.$scope['mqExtra_ProductForm']
    }, {
      default: _withCtx((_ctx) => [
        _createElementVNode("div", { class: "q-my-md" }, [
          _createVNode(_component_m_form, {
            name: "ProductForm",
            id: "ProductForm",
            action: "/Parity/saveProduct",
            "read-only": "",
            "fields-initial": {'productName':_ctx.$mq.str(_ctx.$scope['productName']),'price':_ctx.$mq.str(_ctx.$scope['price']),'note':_ctx.$mq.str(_ctx.$scope['note']),'moquiFormName':'ProductForm'}
          }, {
            default: _withCtx((formProps) => [
              _createVNode(_component_q_list, null, {
                default: _withCtx(() => [
                  _createVNode(_component_q_list, {
                    bordered: "",
                    padding: ""
                  }, {
                    default: _withCtx(() => [
                      _createVNode(_component_q_expansion_item, {
                        "switch-toggle-side": "",
                        group: "ProductForm_accordion",
                        label: _ctx.ec.l10n.localize('Names')
                      }, {
                        default: _withCtx(() => [
                          _createElementVNode("div", { class: "panel-body" }, [
                            _createElementVNode("div", { class: "q-ma-sm" }, [
                              _createVNode(_component_m_text_line, {
                                id: "ProductForm_productName",
                                type: "text",
                                name: "productName",
                                style: {"width":"100%"},
                                label: _ctx.ec.l10n.localize('Product Name'),
                                modelValue: formProps.fields.productName,
                                "onUpdate:modelValue": $event => ((formProps.fields.productName) = $event),
                                fields: formProps.fields
                              }, null, 8, ["label", "modelValue", "onUpdate:modelValue", "fields"])
                            ])
                          ])
                        ]),
                        _: 2
                      }, 1032, ["label"])
                    ]),
                    _: 2
                  }, 1024),
                  _createVNode(_component_q_list, {
                    bordered: "",
                    padding: ""
                  }, {
                    default: _withCtx(() => [
                      _createVNode(_component_q_expansion_item, {
                        "switch-toggle-side": "",
                        group: "ProductForm_accordion",
                        label: _ctx.ec.l10n.localize('Amounts'),
                        "default-opened": ""
                      }, {
                        default: _withCtx(() => [
                          _createElementVNode("div", { class: "panel-body" }, [
                            _createElementVNode("div", { class: "q-ma-sm" }, [
                              _createVNode(_component_m_text_line, {
                                id: "ProductForm_price",
                                type: "text",
                                name: "price",
                                style: {"width":"100%"},
                                label: _ctx.ec.l10n.localize('Price'),
                                modelValue: formProps.fields.price,
                                "onUpdate:modelValue": $event => ((formProps.fields.price) = $event),
                                fields: formProps.fields
                              }, null, 8, ["label", "modelValue", "onUpdate:modelValue", "fields"])
                            ]),
                            _createElementVNode("div", { class: "q-ma-sm" }, [
                              _createVNode(_component_m_text_area, {
                                name: "note",
                                id: "ProductForm_note",
                                rows: "3",
                                style: {"width":"100%"},
                                label: _ctx.ec.l10n.localize('Note'),
                                modelValue: formProps.fields.note,
                                "onUpdate:modelValue": $event => ((formProps.fields.note) = $event)
                              }, null, 8, ["label", "modelValue", "onUpdate:modelValue"])
                            ])
                          ])
                        ]),
                        _: 2
                      }, 1032, ["label"])
                    ]),
                    _: 2
                  }, 1024)
                ]),
                _: 2
              }, 1024)
            ]),
            _: 1
          }, 8, ["fields-initial"])
        ])
      ]),
      _: 1
    }, 8, ["value", "extra"]),
    _createVNode(_component_m_static_form_list, {
      name: "OrderList",
      id: "OrderList",
      action: "/Parity",
      "list-name": "orderLines",
      "skip-form": ""
    }, {
      header: _withCtx((hs) => [
        _createElementVNode("div", { class: "th text-left" }, [
          _createElementVNode("div", null, [
            _createElementVNode("div", { class: "form-title" }, [
              _createElementVNode("span", {
                textContent: _toDisplayString(_ctx.ec.l10n.localize('Order ID'))
              }, null, 8, ["textContent"])
            ])
          ])
        ]),
        _createElementVNode("div", { class: "th text-left" }, [
          _createElementVNode("div", null, [
            _createElementVNode("div", { class: "form-title" }, [
              _createElementVNode("span", {
                textContent: _toDisplayString(_ctx.ec.l10n.localize('Customer'))
              }, null, 8, ["textContent"])
            ])
          ])
        ])
      ]),
      row: _withCtx((_ctx) => [
        _createElementVNode("div", {
          class: _normalizeClass(["tr", {'text-weight-bold': _ctx._moquiRowType === 'total'}])
        }, [
          _createElementVNode("div", { class: "td" }, [
            _createElementVNode("div", { class: "form-group" }, [
              _createVNode(_component_m_display, {
                name: "orderId",
                id: 'OrderList_orderId'+'_'+_ctx.rowIndex+'_display'
              }, {
                default: _withCtx(() => [
                  _createElementVNode("span", {
                    textContent: _toDisplayString(_ctx.$mq.str(_ctx.$scope['orderId']))
                  }, null, 8, ["textContent"])
                ]),
                _: 1
              }, 8, ["id"]),
              _createElementVNode("input", {
                type: "hidden",
                name: "orderId",
                value: _ctx.$mq.str(_ctx.$scope['orderId'])
              }, null, 8, ["value"])
            ])
          ]),
          _createElementVNode("div", { class: "td" }, [
            _createElementVNode("div", { class: "form-group" }, [
              _createVNode(_component_m_display, {
                name: "customer",
                id: 'OrderList_customer'+'_'+_ctx.rowIndex+'_display'
              }, {
                default: _withCtx(() => [
                  _createElementVNode("span", {
                    textContent: _toDisplayString(_ctx.$mq.str(_ctx.$scope['customer']))
                  }, null, 8, ["textContent"])
                ]),
                _: 1
              }, 8, ["id"]),
              _createElementVNode("input", {
                type: "hidden",
                name: "customer",
                value: _ctx.$mq.str(_ctx.$scope['customer'])
              }, null, 8, ["value"])
            ])
          ])
        ], 2)
      ]),
      subhead: _withCtx(() => [
        _createElementVNode("div", { class: "col" }, [
          _createElementVNode("span", {
            textContent: _toDisplayString(_ctx.ec.l10n.localize('Item'))
          }, null, 8, ["textContent"])
        ]),
        _createElementVNode("div", { class: "col" }, [
          _createElementVNode("span", {
            textContent: _toDisplayString(_ctx.ec.l10n.localize('Qty'))
          }, null, 8, ["textContent"])
        ])
      ]),
      subrow: _withCtx((_ctx) => [
        _createElementVNode("div", { class: "col" }, [
          _createElementVNode("div", { class: "form-group" }, [
            _createVNode(_component_m_display, {
              name: "item",
              id: 'OrderList_item'+'_'+_ctx.rowIndex+'_display'
            }, {
              default: _withCtx(() => [
                _createElementVNode("span", {
                  textContent: _toDisplayString(_ctx.$mq.str(_ctx.$scope['item']))
                }, null, 8, ["textContent"])
              ]),
              _: 1
            }, 8, ["id"]),
            _createElementVNode("input", {
              type: "hidden",
              name: "item",
              value: _ctx.$mq.str(_ctx.$scope['item'])
            }, null, 8, ["value"])
          ])
        ]),
        _createElementVNode("div", { class: "col" }, [
          _createElementVNode("div", { class: "form-group" }, [
            _createVNode(_component_m_display, {
              name: "qty",
              id: 'OrderList_qty'+'_'+_ctx.rowIndex+'_display'
            }, {
              default: _withCtx(() => [
                _createElementVNode("span", {
                  textContent: _toDisplayString(_ctx.$mq.str(_ctx.$scope['qty']))
                }, null, 8, ["textContent"])
              ]),
              _: 1
            }, 8, ["id"]),
            _createElementVNode("input", {
              type: "hidden",
              name: "qty",
              value: _ctx.$mq.str(_ctx.$scope['qty'])
            }, null, 8, ["value"])
          ])
        ])
      ]),
      _: 1
    }),
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
