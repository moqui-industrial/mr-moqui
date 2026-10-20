/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

// Generated from the component templates of WebrootVue.qvt.js; do not edit. Make it again with
//   UPDATE_GOLDEN=1 cargo test -p moqui-core --lib client_components_are_precompiled
// qvt.applyRenders uses an entry only while its hash matches the template (plans/archive/QvtServerStatic.md, Page build).
(function (moqui) {
    "use strict";
    moqui.qvtRenders = {
        "m-avatar": { hash: "ad2eb49a", make: function (Vue) {
            const { openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, toDisplayString: _toDisplayString, resolveComponent: _resolveComponent, createBlock: _createBlock, normalizeClass: _normalizeClass, createTextVNode: _createTextVNode, withCtx: _withCtx } = Vue

            return function render(_ctx, _cache) {
              const _component_q_icon = _resolveComponent("q-icon")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_avatar = _resolveComponent("q-avatar")

              return (_openBlock(), _createBlock(_component_q_avatar, {
                class: "m-avatar",
                size: _ctx.size,
                color: _ctx.background,
                "text-color": _ctx.showImage ? undefined : _ctx.textColor,
                square: _ctx.square,
                rounded: _ctx.rounded,
                "aria-label": _ctx.name || undefined
              }, {
                default: _withCtx(() => [
                  (_ctx.showImage)
                    ? (_openBlock(), _createElementBlock("img", {
                        key: 0,
                        src: _ctx.image,
                        alt: _ctx.name || '',
                        onError: $event => (_ctx.failedImage = _ctx.image)
                      }, null, 40, ["src", "alt", "onError"]))
                    : (_ctx.initials)
                      ? (_openBlock(), _createElementBlock("span", {
                          key: 1,
                          class: "m-avatar-initials"
                        }, _toDisplayString(_ctx.initials), 1))
                      : (_openBlock(), _createBlock(_component_q_icon, {
                          key: 2,
                          name: "sym_o_person"
                        })),
                  (_ctx.shownPresence)
                    ? (_openBlock(), _createElementBlock("span", {
                        key: 3,
                        class: _normalizeClass(["m-avatar-presence", 'm-presence-' + _ctx.shownPresence]),
                        title: _ctx.shownPresence
                      }, null, 10, ["title"]))
                    : _createCommentVNode("", true),
                  (_ctx.tooltip && _ctx.name)
                    ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 4 }, {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(_ctx.name), 1)
                        ]),
                        _: 1
                      }))
                    : _createCommentVNode("", true)
                ]),
                _: 1
              }, 8, ["size", "color", "text-color", "square", "rounded", "aria-label"]))
            }
        } },
        "m-box-body": { hash: "4645e5c2", make: function (Vue) {
            const { renderSlot: _renderSlot, normalizeStyle: _normalizeStyle, openBlock: _openBlock, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createElementBlock("div", {
                class: "q-pa-xs",
                style: _normalizeStyle(_ctx.bodyStyle)
              }, [
                _renderSlot(_ctx.$slots, "default")
              ], 4))
            }
        } },
        "m-chart": { hash: "e1d8e95a", make: function (Vue) {
            const { createElementVNode: _createElementVNode, normalizeStyle: _normalizeStyle, openBlock: _openBlock, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createElementBlock("div", {
                class: "chart-container",
                style: _normalizeStyle([{"position":"relative"}, {height:_ctx.height,width:_ctx.width}])
              }, [
                _createElementVNode("canvas", { ref: "canvas" }, null, 512)
              ], 4))
            }
        } },
        "m-chatter": { hash: "8482661e", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, normalizeClass: _normalizeClass, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode } = Vue

            return function render(_ctx, _cache) {
              const _component_m_dynamic_container = _resolveComponent("m-dynamic-container")

              return (_ctx.loadUrl)
                ? (_openBlock(), _createElementBlock("div", {
                    key: 0,
                    class: _normalizeClass('m-record-panel m-record-panel-' + _ctx.kind)
                  }, [
                    _createVNode(_component_m_dynamic_container, {
                      ref: "container",
                      id: _ctx.containerId,
                      url: _ctx.loadUrl
                    }, null, 8, ["id", "url"])
                  ], 2))
                : _createCommentVNode("", true)
            }
        } },
        "m-check": { hash: "2b541cd8", make: function (Vue) {
            const { resolveComponent: _resolveComponent, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, withCtx: _withCtx, createVNode: _createVNode, mergeProps: _mergeProps } = Vue

            return function render(_ctx, _cache) {
              const _component_q_toggle = _resolveComponent("q-toggle")
              const _component_q_checkbox = _resolveComponent("q-checkbox")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_option_group = _resolveComponent("q-option-group")
              const _component_m_field = _resolveComponent("m-field")

              return (!_ctx.options.length && _ctx.toggle)
                ? (_openBlock(), _createBlock(_component_q_toggle, {
                    key: 0,
                    dense: "",
                    size: "xs",
                    "model-value": _ctx.local,
                    "onUpdate:modelValue": _ctx.onInput,
                    "true-value": _ctx.trueValue,
                    "false-value": _ctx.falseValue,
                    label: _ctx.singleLabel,
                    name: _ctx.name,
                    id: _ctx.id,
                    form: _ctx.form,
                    disable: _ctx.disable
                  }, null, 8, ["model-value", "onUpdate:modelValue", "true-value", "false-value", "label", "name", "id", "form", "disable"]))
                : (!_ctx.options.length)
                  ? (_openBlock(), _createBlock(_component_q_checkbox, {
                      key: 1,
                      dense: "",
                      size: "xs",
                      "model-value": _ctx.local,
                      "onUpdate:modelValue": _ctx.onInput,
                      "true-value": _ctx.trueValue,
                      "false-value": _ctx.falseValue,
                      label: _ctx.singleLabel,
                      name: _ctx.name,
                      id: _ctx.id,
                      form: _ctx.form,
                      disable: _ctx.disable
                    }, null, 8, ["model-value", "onUpdate:modelValue", "true-value", "false-value", "label", "name", "id", "form", "disable"]))
                  : (!_ctx.noWrapper)
                    ? (_openBlock(), _createBlock(_component_m_field, _mergeProps({
                        key: 2,
                        borderless: "",
                        class: "m-field-group",
                        label: _ctx.label,
                        "bg-color": _ctx.bg,
                        disable: _ctx.disable
                      }, _ctx.errorAttrs), {
                        default: _withCtx(() => [
                          (_ctx.tooltip)
                            ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                                default: _withCtx(() => [
                                  _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                                ]),
                                _: 1
                              }))
                            : _createCommentVNode("", true),
                          _createVNode(_component_q_option_group, {
                            type: "checkbox",
                            inline: "",
                            dense: "",
                            size: "sm",
                            options: _ctx.options,
                            "model-value": _ctx.local,
                            "onUpdate:modelValue": _ctx.onInput,
                            name: _ctx.name,
                            id: _ctx.id,
                            disable: _ctx.disable
                          }, null, 8, ["options", "model-value", "onUpdate:modelValue", "name", "id", "disable"])
                        ]),
                        _: 1
                      }, 16, ["label", "bg-color", "disable"]))
                    : (_openBlock(), _createBlock(_component_q_option_group, {
                        key: 3,
                        type: "checkbox",
                        inline: "",
                        dense: "",
                        size: "sm",
                        options: _ctx.options,
                        "model-value": _ctx.local,
                        "onUpdate:modelValue": _ctx.onInput,
                        name: _ctx.name,
                        id: _ctx.id,
                        disable: _ctx.disable
                      }, null, 8, ["options", "model-value", "onUpdate:modelValue", "name", "id", "disable"]))
            }
        } },
        "m-checkbox-set": { hash: "94e2c135", make: function (Vue) {
            const { renderSlot: _renderSlot, openBlock: _openBlock, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createElementBlock("span", { class: "checkbox-set" }, [
                _renderSlot(_ctx.$slots, "default", {
                  checkboxAllState: _ctx.checkboxAllState,
                  setCheckboxAllState: _ctx.setCheckboxAllState,
                  checkboxStates: _ctx.checkboxStates,
                  clickCheckbox: _ctx.clickCheckbox,
                  addCheckboxParameters: _ctx.addCheckboxParameters
                })
              ]))
            }
        } },
        "m-container-box": { hash: "faa2a157", make: function (Vue) {
            const { toDisplayString: _toDisplayString, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, renderSlot: _renderSlot, resolveComponent: _resolveComponent, createVNode: _createVNode, withModifiers: _withModifiers, createElementVNode: _createElementVNode, withCtx: _withCtx, vShow: _vShow, withDirectives: _withDirectives, normalizeClass: _normalizeClass, createBlock: _createBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_space = _resolveComponent("q-space")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_separator = _resolveComponent("q-separator")
              const _component_q_card_section = _resolveComponent("q-card-section")
              const _component_q_slide_transition = _resolveComponent("q-slide-transition")
              const _component_q_card = _resolveComponent("q-card")

              return (_openBlock(), _createBlock(_component_q_card, {
                flat: "",
                bordered: "",
                class: _normalizeClass(["q-ma-sm m-container-box", _ctx.boxClass])
              }, {
                default: _withCtx(() => [
                  _createElementVNode("div", {
                    class: "m-box-head row items-center no-wrap q-py-xs q-px-sm",
                    onClick: _withModifiers(_ctx.toggleBody, ["self"])
                  }, [
                    (_ctx.title && _ctx.title.length)
                      ? (_openBlock(), _createElementBlock("div", {
                          key: 0,
                          class: "text-subtitle1 text-weight-medium ellipsis cursor-pointer",
                          onClick: _ctx.toggleBody
                        }, _toDisplayString(_ctx.title), 9, ["onClick"]))
                      : _createCommentVNode("", true),
                    _renderSlot(_ctx.$slots, "header"),
                    _createVNode(_component_q_space),
                    _renderSlot(_ctx.$slots, "toolbar"),
                    _createVNode(_component_q_btn, {
                      flat: "",
                      round: "",
                      dense: "",
                      size: "sm",
                      color: "grey-7",
                      icon: _ctx.isBodyOpen ? 'expand_less' : 'expand_more',
                      onClick: _ctx.toggleBody
                    }, null, 8, ["icon", "onClick"])
                  ], 8, ["onClick"]),
                  _createVNode(_component_q_slide_transition, null, {
                    default: _withCtx(() => [
                      _withDirectives(_createElementVNode("div", null, [
                        _createVNode(_component_q_separator),
                        _createVNode(_component_q_card_section, null, {
                          default: _withCtx(() => [
                            _renderSlot(_ctx.$slots, "default")
                          ]),
                          _: 3
                        })
                      ], 512), [
                        [_vShow, _ctx.isBodyOpen]
                      ])
                    ]),
                    _: 3
                  })
                ]),
                _: 3
              }, 8, ["class"]))
            }
        } },
        "m-container-dialog": { hash: "416fea64", make: function (Vue) {
            const { renderSlot: _renderSlot, resolveComponent: _resolveComponent, normalizeClass: _normalizeClass, createVNode: _createVNode, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, withCtx: _withCtx, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createElementVNode: _createElementVNode, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_m_dialog = _resolveComponent("m-dialog")

              return (_openBlock(), _createElementBlock("span", null, [
                _createElementVNode("span", {
                  onClick: $event => (_ctx.show())
                }, [
                  _renderSlot(_ctx.$slots, "button", {}, () => [
                    _createVNode(_component_q_btn, {
                      dense: "",
                      unelevated: "",
                      "no-caps": "",
                      icon: _ctx.buttonIcon,
                      label: _ctx.buttonText,
                      class: _normalizeClass(_ctx.moqui.tonalClass(_ctx.color, _ctx.buttonClass))
                    }, null, 8, ["icon", "label", "class"])
                  ]),
                  (_ctx.tooltip)
                    ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                        ]),
                        _: 1
                      }))
                    : _createCommentVNode("", true)
                ], 8, ["onClick"]),
                _createVNode(_component_m_dialog, {
                  modelValue: _ctx.isShown,
                  "onUpdate:modelValue": $event => ((_ctx.isShown) = $event),
                  id: _ctx.id,
                  title: _ctx.title,
                  color: _ctx.color,
                  width: _ctx.width
                }, {
                  default: _withCtx(() => [
                    _renderSlot(_ctx.$slots, "default")
                  ]),
                  _: 3
                }, 8, ["modelValue", "onUpdate:modelValue", "id", "title", "color", "width"])
              ]))
            }
        } },
        "m-container-split": { hash: "7ec2b534", make: function (Vue) {
            const { resolveComponent: _resolveComponent, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, renderSlot: _renderSlot, vShow: _vShow, withDirectives: _withDirectives, createElementBlock: _createElementBlock, normalizeStyle: _normalizeStyle, withCtx: _withCtx, createVNode: _createVNode, normalizeClass: _normalizeClass } = Vue

            return function render(_ctx, _cache) {
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_splitter = _resolveComponent("q-splitter")

              return (_openBlock(), _createElementBlock("div", {
                ref: "root",
                class: _normalizeClass(["m-container-split", {'m-split-stacked': _ctx.layout.stacked, 'm-split-right': _ctx.right, 'm-split-selected': _ctx.selected}]),
                id: _ctx.id,
                "data-stacked": _ctx.layout.stacked ? 'true' : 'false'
              }, [
                _createVNode(_component_q_splitter, {
                  "model-value": _ctx.layout.side,
                  "onUpdate:modelValue": _ctx.drag,
                  unit: "px",
                  limits: [_ctx.layout.min, _ctx.layout.max],
                  reverse: _ctx.right,
                  disable: _ctx.layout.stacked,
                  style: _normalizeStyle(_ctx.paneHeight ? {height: _ctx.paneHeight} : undefined),
                  "separator-class": "m-split-separator"
                }, {
                  before: _withCtx(() => [
                    (_ctx.right)
                      ? _withDirectives((_openBlock(), _createElementBlock("div", {
                          key: 0,
                          class: "m-split-main"
                        }, [
                          (_ctx.backShown)
                            ? (_openBlock(), _createBlock(_component_q_btn, {
                                key: 0,
                                flat: "",
                                dense: "",
                                "no-caps": "",
                                icon: "arrow_back",
                                label: _ctx.backLabel,
                                class: "m-split-back",
                                onClick: _ctx.back
                              }, null, 8, ["label", "onClick"]))
                            : _createCommentVNode("", true),
                          _renderSlot(_ctx.$slots, "main")
                        ], 512)), [
                          [_vShow, _ctx.showMain]
                        ])
                      : _withDirectives((_openBlock(), _createElementBlock("div", {
                          key: 1,
                          class: "m-split-side"
                        }, [
                          _renderSlot(_ctx.$slots, "side")
                        ], 512)), [
                          [_vShow, _ctx.showSide]
                        ])
                  ]),
                  after: _withCtx(() => [
                    (_ctx.right)
                      ? _withDirectives((_openBlock(), _createElementBlock("div", {
                          key: 0,
                          class: "m-split-side"
                        }, [
                          _renderSlot(_ctx.$slots, "side")
                        ], 512)), [
                          [_vShow, _ctx.showSide]
                        ])
                      : _withDirectives((_openBlock(), _createElementBlock("div", {
                          key: 1,
                          class: "m-split-main"
                        }, [
                          (_ctx.backShown)
                            ? (_openBlock(), _createBlock(_component_q_btn, {
                                key: 0,
                                flat: "",
                                dense: "",
                                "no-caps": "",
                                icon: "arrow_back",
                                label: _ctx.backLabel,
                                class: "m-split-back",
                                onClick: _ctx.back
                              }, null, 8, ["label", "onClick"]))
                            : _createCommentVNode("", true),
                          _renderSlot(_ctx.$slots, "main")
                        ], 512)), [
                          [_vShow, _ctx.showMain]
                        ])
                  ]),
                  _: 3
                }, 8, ["model-value", "onUpdate:modelValue", "limits", "reverse", "disable", "style"])
              ], 10, ["id", "data-stacked"]))
            }
        } },
        "m-date-period": { hash: "9a3df286", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, createTextVNode: _createTextVNode, withCtx: _withCtx, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, toDisplayString: _toDisplayString, createBlock: _createBlock, createElementVNode: _createElementVNode } = Vue

            return function render(_ctx, _cache) {
              const _component_m_date_time = _resolveComponent("m-date-time")
              const _component_q_icon = _resolveComponent("q-icon")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_date = _resolveComponent("q-date")
              const _component_q_popup_proxy = _resolveComponent("q-popup-proxy")
              const _component_q_select = _resolveComponent("q-select")
              const _component_q_menu = _resolveComponent("q-menu")
              const _component_q_input = _resolveComponent("q-input")

              return (_ctx.fromThruMode)
                ? (_openBlock(), _createElementBlock("div", {
                    key: 0,
                    class: "row"
                  }, [
                    _createVNode(_component_m_date_time, {
                      name: _ctx.name+'_from',
                      id: _ctx.id+'_from',
                      label: _ctx.label+' From',
                      form: _ctx.form,
                      type: _ctx.fromThruType,
                      modelValue: _ctx.fields[_ctx.name+'_from'],
                      "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name+'_from']) = $event)
                    }, null, 8, ["name", "id", "label", "form", "type", "modelValue", "onUpdate:modelValue"]),
                    _createVNode(_component_q_icon, {
                      class: "q-my-auto",
                      name: "remove"
                    }),
                    _createVNode(_component_m_date_time, {
                      name: _ctx.name+'_thru',
                      id: _ctx.id+'_thru',
                      label: _ctx.label+' Thru',
                      form: _ctx.form,
                      type: _ctx.fromThruType,
                      modelValue: _ctx.fields[_ctx.name+'_thru'],
                      "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name+'_thru']) = $event)
                    }, {
                      after: _withCtx(() => [
                        _createVNode(_component_q_btn, {
                          flat: "",
                          round: "",
                          dense: "",
                          size: "sm",
                          icon: "calendar_view_day",
                          onClick: _ctx.toggleMode
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_tooltip, null, {
                              default: _withCtx(() => [
                                _createTextVNode("Period Select Mode")
                              ]),
                              _: 1
                            })
                          ]),
                          _: 1
                        }, 8, ["onClick"]),
                        _createVNode(_component_q_btn, {
                          flat: "",
                          round: "",
                          dense: "",
                          size: "sm",
                          icon: "clear",
                          onClick: _ctx.clearAll
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_tooltip, null, {
                              default: _withCtx(() => [
                                _createTextVNode("Clear")
                              ]),
                              _: 1
                            })
                          ]),
                          _: 1
                        }, 8, ["onClick"])
                      ]),
                      _: 1
                    }, 8, ["name", "id", "label", "form", "type", "modelValue", "onUpdate:modelValue"])
                  ]))
                : (_openBlock(), _createElementBlock("div", {
                    key: 1,
                    class: "row"
                  }, [
                    _createVNode(_component_q_input, {
                      dense: "",
                      outlined: "",
                      "stack-label": "",
                      "hide-bottom-space": "",
                      label: _ctx.label,
                      modelValue: _ctx.fields[_ctx.name+'_pdate'],
                      "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name+'_pdate']) = $event),
                      mask: "####-##-##",
                      "fill-mask": "",
                      for: _ctx.id,
                      name: _ctx.name+'_pdate',
                      form: _ctx.form,
                      style: {"max-width":"max-content"},
                      "bg-color": _ctx.changed(_ctx.name+'_pdate')
                    }, {
                      prepend: _withCtx(() => [
                        _createVNode(_component_q_icon, {
                          name: "event",
                          size: "xs",
                          class: "cursor-pointer"
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_popup_proxy, {
                              ref: "qDateProxy",
                              cover: "",
                              "transition-show": "scale",
                              "transition-hide": "scale"
                            }, {
                              default: _withCtx(() => [
                                _createVNode(_component_q_date, {
                                  modelValue: _ctx.fields[_ctx.name+'_pdate'],
                                  "onUpdate:modelValue": [$event => ((_ctx.fields[_ctx.name+'_pdate']) = $event), _ctx.hideDate],
                                  mask: "YYYY-MM-DD",
                                  "today-btn": ""
                                }, null, 8, ["modelValue", "onUpdate:modelValue"])
                              ]),
                              _: 1
                            }, 512)
                          ]),
                          _: 1
                        })
                      ]),
                      append: _withCtx(() => [
                        _createVNode(_component_q_btn, {
                          flat: "",
                          dense: "",
                          "no-caps": "",
                          size: "sm",
                          icon: "event_repeat",
                          round: !_ctx.periodText,
                          label: _ctx.periodText || undefined,
                          color: _ctx.periodText ? 'primary' : 'grey-6'
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_tooltip, null, {
                              default: _withCtx(() => [
                                _createTextVNode("Period")
                              ]),
                              _: 1
                            }),
                            _createVNode(_component_q_menu, { class: "m-date-period-options q-pa-sm" }, {
                              default: _withCtx(() => [
                                _createElementVNode("div", { class: "column q-gutter-y-xs" }, [
                                  _createVNode(_component_q_select, {
                                    dense: "",
                                    outlined: "",
                                    "options-dense": "",
                                    "emit-value": "",
                                    "map-options": "",
                                    "hide-bottom-space": "",
                                    modelValue: _ctx.fields[_ctx.name+'_period'],
                                    "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name+'_period']) = $event),
                                    name: _ctx.name+'_period',
                                    "bg-color": _ctx.changed(_ctx.name+'_period'),
                                    "stack-label": "",
                                    label: "Period",
                                    options: _ctx.datePeriods,
                                    form: _ctx.form,
                                    behavior: "menu"
                                  }, null, 8, ["modelValue", "onUpdate:modelValue", "name", "bg-color", "options", "form"]),
                                  _createVNode(_component_q_select, {
                                    dense: "",
                                    outlined: "",
                                    "options-dense": "",
                                    "emit-value": "",
                                    "map-options": "",
                                    "hide-bottom-space": "",
                                    modelValue: _ctx.fields[_ctx.name+'_poffset'],
                                    "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name+'_poffset']) = $event),
                                    name: _ctx.name+'_poffset',
                                    "bg-color": _ctx.changed(_ctx.name+'_poffset'),
                                    "stack-label": "",
                                    label: "Offset",
                                    options: _ctx.dateOffsets,
                                    form: _ctx.form,
                                    behavior: "menu"
                                  }, null, 8, ["modelValue", "onUpdate:modelValue", "name", "bg-color", "options", "form"])
                                ])
                              ]),
                              _: 1
                            })
                          ]),
                          _: 1
                        }, 8, ["round", "label", "color"])
                      ]),
                      after: _withCtx(() => [
                        _createVNode(_component_q_btn, {
                          flat: "",
                          round: "",
                          dense: "",
                          size: "sm",
                          icon: "date_range",
                          onClick: _ctx.toggleMode
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_tooltip, null, {
                              default: _withCtx(() => [
                                _createTextVNode("Date Range Mode")
                              ]),
                              _: 1
                            })
                          ]),
                          _: 1
                        }, 8, ["onClick"]),
                        _createVNode(_component_q_btn, {
                          flat: "",
                          round: "",
                          dense: "",
                          size: "sm",
                          icon: "clear",
                          onClick: _ctx.clearAll
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_tooltip, null, {
                              default: _withCtx(() => [
                                _createTextVNode("Clear")
                              ]),
                              _: 1
                            })
                          ]),
                          _: 1
                        }, 8, ["onClick"])
                      ]),
                      default: _withCtx(() => [
                        (_ctx.tooltip)
                          ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                              default: _withCtx(() => [
                                _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                              ]),
                              _: 1
                            }))
                          : _createCommentVNode("", true)
                      ]),
                      _: 1
                    }, 8, ["label", "modelValue", "onUpdate:modelValue", "for", "name", "form", "bg-color"])
                  ]))
            }
        } },
        "m-date-time": { hash: "94ef7446", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, withCtx: _withCtx, renderSlot: _renderSlot, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, mergeProps: _mergeProps, createSlots: _createSlots } = Vue

            return function render(_ctx, _cache) {
              const _component_q_date = _resolveComponent("q-date")
              const _component_q_popup_proxy = _resolveComponent("q-popup-proxy")
              const _component_q_icon = _resolveComponent("q-icon")
              const _component_q_time = _resolveComponent("q-time")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_input = _resolveComponent("q-input")

              return (_openBlock(), _createBlock(_component_q_input, _mergeProps({
                dense: "",
                outlined: "",
                "stack-label": "",
                "hide-bottom-space": "",
                "no-error-icon": "",
                label: _ctx.label,
                "model-value": _ctx.local,
                "onUpdate:modelValue": _ctx.onInput,
                rules: _ctx.allRules,
                mask: _ctx.inputMask,
                "fill-mask": "",
                for: _ctx.id,
                name: _ctx.name,
                form: _ctx.form,
                disable: _ctx.disable,
                size: _ctx.sizeVal,
                style: {"max-width":"max-content"},
                "bg-color": _ctx.bg
              }, _ctx.errorAttrs), _createSlots({
                after: _withCtx(() => [
                  _renderSlot(_ctx.$slots, "after")
                ]),
                default: _withCtx(() => [
                  (_ctx.tooltip)
                    ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                        ]),
                        _: 1
                      }))
                    : _createCommentVNode("", true)
                ]),
                _: 2
              }, [
                (_ctx.hasDate)
                  ? {
                      name: "prepend",
                      fn: _withCtx(() => [
                        _createVNode(_component_q_icon, {
                          name: "event",
                          size: "xs",
                          class: "cursor-pointer"
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_popup_proxy, {
                              ref: "qDateProxy",
                              cover: "",
                              "transition-show": "scale",
                              "transition-hide": "scale"
                            }, {
                              default: _withCtx(() => [
                                _createVNode(_component_q_date, {
                                  "model-value": _ctx.local,
                                  "onUpdate:modelValue": _ctx.onDate,
                                  mask: _ctx.formatVal,
                                  "today-btn": ""
                                }, null, 8, ["model-value", "onUpdate:modelValue", "mask"])
                              ]),
                              _: 1
                            }, 512)
                          ]),
                          _: 1
                        })
                      ]),
                      key: "0"
                    }
                  : undefined,
                (_ctx.hasTime)
                  ? {
                      name: "append",
                      fn: _withCtx(() => [
                        _createVNode(_component_q_icon, {
                          name: "access_time",
                          size: "xs",
                          class: "cursor-pointer"
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_popup_proxy, {
                              ref: "qTimeProxy",
                              cover: "",
                              "transition-show": "scale",
                              "transition-hide": "scale"
                            }, {
                              default: _withCtx(() => [
                                _createVNode(_component_q_time, {
                                  "model-value": _ctx.local,
                                  "onUpdate:modelValue": _ctx.onTime,
                                  mask: _ctx.formatVal,
                                  format24h: "",
                                  "now-btn": ""
                                }, null, 8, ["model-value", "onUpdate:modelValue", "mask"])
                              ]),
                              _: 1
                            }, 512)
                          ]),
                          _: 1
                        })
                      ]),
                      key: "1"
                    }
                  : undefined
              ]), 1040, ["label", "model-value", "onUpdate:modelValue", "rules", "mask", "for", "name", "form", "disable", "size", "bg-color"]))
            }
        } },
        "m-dialog": { hash: "353c53eb", make: function (Vue) {
            const { toDisplayString: _toDisplayString, createElementVNode: _createElementVNode, resolveComponent: _resolveComponent, createVNode: _createVNode, resolveDirective: _resolveDirective, withDirectives: _withDirectives, normalizeStyle: _normalizeStyle, withCtx: _withCtx, renderSlot: _renderSlot, openBlock: _openBlock, createBlock: _createBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_space = _resolveComponent("q-space")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_card_section = _resolveComponent("q-card-section")
              const _component_q_separator = _resolveComponent("q-separator")
              const _component_q_card = _resolveComponent("q-card")
              const _component_q_dialog = _resolveComponent("q-dialog")
              const _directive_close_popup = _resolveDirective("close-popup")

              return (_openBlock(), _createBlock(_component_q_dialog, {
                "model-value": _ctx.modelValue,
                "onUpdate:modelValue": $event => (_ctx.$emit('update:modelValue', $event)),
                id: _ctx.id,
                onShow: _ctx.onShow,
                onHide: _ctx.onHide,
                maximized: _ctx.$q.screen.lt.sm
              }, {
                default: _withCtx(() => [
                  _createVNode(_component_q_card, {
                    ref: "dialogCard",
                    flat: "",
                    bordered: "",
                    style: _normalizeStyle({width:((_ctx.width||760)+'px'),'max-width':(_ctx.$q.screen.lt.sm?'100vw':'90vw')})
                  }, {
                    default: _withCtx(() => [
                      _createVNode(_component_q_card_section, {
                        ref: "dialogHeader",
                        class: "row items-center no-wrap q-py-xs q-pr-xs non-selectable",
                        style: _normalizeStyle({cursor:(_ctx.draggable?'move':'default')})
                      }, {
                        default: _withCtx(() => [
                          _createElementVNode("div", { class: "text-subtitle1 text-weight-medium ellipsis" }, _toDisplayString(_ctx.title), 1),
                          _createVNode(_component_q_space),
                          _withDirectives(_createVNode(_component_q_btn, {
                            icon: "close",
                            flat: "",
                            round: "",
                            dense: ""
                          }, null, 512), [
                            [_directive_close_popup]
                          ])
                        ]),
                        _: 1
                      }, 8, ["style"]),
                      _createVNode(_component_q_separator),
                      _createVNode(_component_q_card_section, {
                        ref: "dialogBody",
                        class: "scroll",
                        style: {"max-height":"80vh"}
                      }, {
                        default: _withCtx(() => [
                          _renderSlot(_ctx.$slots, "default")
                        ]),
                        _: 3
                      }, 512)
                    ]),
                    _: 3
                  }, 8, ["style"])
                ]),
                _: 3
              }, 8, ["model-value", "onUpdate:modelValue", "id", "onShow", "onHide", "maximized"]))
            }
        } },
        "m-display": { hash: "ccb7f263", make: function (Vue) {
            const { toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, renderSlot: _renderSlot, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_input = _resolveComponent("q-input")

              return (_ctx.labelWrapper)
                ? (_openBlock(), _createBlock(_component_q_input, {
                    key: 0,
                    dense: "",
                    outlined: "",
                    readonly: "",
                    "stack-label": "",
                    autogrow: "",
                    "model-value": _ctx.displayValue,
                    label: _ctx.label,
                    for: _ctx.id,
                    name: _ctx.name,
                    loading: _ctx.loading
                  }, {
                    default: _withCtx(() => [
                      (_ctx.tooltip)
                        ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                            default: _withCtx(() => [
                              _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                            ]),
                            _: 1
                          }))
                        : _createCommentVNode("", true)
                    ]),
                    _: 1
                  }, 8, ["model-value", "label", "for", "name", "loading"]))
                : (_openBlock(), _createElementBlock("span", {
                    key: 1,
                    id: _ctx.id
                  }, [
                    (_ctx.tooltip)
                      ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                          default: _withCtx(() => [
                            _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                          ]),
                          _: 1
                        }))
                      : _createCommentVNode("", true),
                    _renderSlot(_ctx.$slots, "default"),
                    _createTextVNode(_toDisplayString(_ctx.displayValue), 1)
                  ], 8, ["id"]))
            }
        } },
        "m-drop-down": { hash: "5e94df8d", make: function (Vue) {
            const { toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createVNode: _createVNode, renderSlot: _renderSlot, mergeProps: _mergeProps, createSlots: _createSlots } = Vue

            return function render(_ctx, _cache) {
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_item_section = _resolveComponent("q-item-section")
              const _component_q_item = _resolveComponent("q-item")
              const _component_q_select = _resolveComponent("q-select")

              return (_openBlock(), _createBlock(_component_q_select, _mergeProps({
                ref: "qSelect",
                "model-value": _ctx.modelValue,
                "onUpdate:modelValue": _ctx.handleInput,
                dense: "",
                outlined: "",
                "options-dense": "",
                "use-input": "",
                "use-chips": _ctx.multiple,
                "fill-input": !_ctx.multiple,
                "hide-selected": !_ctx.multiple,
                name: _ctx.name,
                for: _ctx.id,
                form: _ctx.form,
                "input-debounce": _ctx.serverSearch ? _ctx.serverDelay : 100,
                onFilter: _ctx.filterFn,
                clearable: _ctx.allowEmpty||_ctx.multiple,
                disable: _ctx.disable,
                multiple: _ctx.multiple,
                "emit-value": !_ctx.onSelectGoTo,
                "map-options": "",
                behavior: "menu",
                "new-value-mode": _ctx.combo ? 'add-unique' : undefined,
                rules: [val => _ctx.allowEmpty||_ctx.multiple||_ctx.combo||val===''||(val&&val.length)||'Please select an option'],
                "stack-label": "",
                "hide-bottom-space": "",
                "no-error-icon": "",
                label: _ctx.label,
                loading: _ctx.loading,
                "bg-color": _ctx.bg,
                options: _ctx.curOptions
              }, _ctx.errorAttrs), _createSlots({
                "no-option": _withCtx(() => [
                  _createVNode(_component_q_item, null, {
                    default: _withCtx(() => [
                      _createVNode(_component_q_item_section, { class: "text-grey" }, {
                        default: _withCtx(() => [
                          _createTextVNode("No results")
                        ]),
                        _: 1
                      })
                    ]),
                    _: 1
                  })
                ]),
                default: _withCtx(() => [
                  (_ctx.tooltip)
                    ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                        ]),
                        _: 1
                      }))
                    : _createCommentVNode("", true)
                ]),
                _: 2
              }, [
                (_ctx.$slots.after)
                  ? {
                      name: "after",
                      fn: _withCtx(() => [
                        _renderSlot(_ctx.$slots, "after")
                      ]),
                      key: "0"
                    }
                  : undefined
              ]), 1040, ["model-value", "onUpdate:modelValue", "use-chips", "fill-input", "hide-selected", "name", "for", "form", "input-debounce", "onFilter", "clearable", "disable", "multiple", "emit-value", "new-value-mode", "rules", "label", "loading", "bg-color", "options"]))
            }
        } },
        "m-dynamic-container": { hash: "70ca5338", make: function (Vue) {
            const { resolveDynamicComponent: _resolveDynamicComponent, openBlock: _openBlock, createBlock: _createBlock } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createBlock(_resolveDynamicComponent(_ctx.curComponent), { ref: "screen" }, null, 512))
            }
        } },
        "m-dynamic-dialog": { hash: "9c4723b0", make: function (Vue) {
            const { resolveComponent: _resolveComponent, normalizeClass: _normalizeClass, createVNode: _createVNode, resolveDynamicComponent: _resolveDynamicComponent, openBlock: _openBlock, createBlock: _createBlock, withCtx: _withCtx, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_m_dialog = _resolveComponent("m-dialog")

              return (_openBlock(), _createElementBlock("span", null, [
                _createVNode(_component_q_btn, {
                  dense: "",
                  unelevated: "",
                  "no-caps": "",
                  icon: _ctx.buttonIcon,
                  label: _ctx.buttonText,
                  class: _normalizeClass(_ctx.moqui.tonalClass(_ctx.color, _ctx.buttonClass)),
                  onClick: $event => (_ctx.isShown = true)
                }, null, 8, ["icon", "label", "class", "onClick"]),
                _createVNode(_component_m_dialog, {
                  ref: "dialog",
                  modelValue: _ctx.isShown,
                  "onUpdate:modelValue": $event => ((_ctx.isShown) = $event),
                  id: _ctx.id,
                  title: _ctx.title,
                  color: _ctx.color,
                  width: _ctx.width
                }, {
                  default: _withCtx(() => [
                    (_openBlock(), _createBlock(_resolveDynamicComponent(_ctx.curComponent)))
                  ]),
                  _: 1
                }, 8, ["modelValue", "onUpdate:modelValue", "id", "title", "color", "width"])
              ]))
            }
        } },
        "m-editable": { hash: "49667c8e", make: function (Vue) {
            const { toDisplayString: _toDisplayString, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, resolveComponent: _resolveComponent, createBlock: _createBlock, createVNode: _createVNode, withKeys: _withKeys, withCtx: _withCtx, resolveDynamicComponent: _resolveDynamicComponent, normalizeClass: _normalizeClass } = Vue

            return function render(_ctx, _cache) {
              const _component_q_select = _resolveComponent("q-select")
              const _component_q_date = _resolveComponent("q-date")
              const _component_q_time = _resolveComponent("q-time")
              const _component_q_input = _resolveComponent("q-input")
              const _component_q_popup_edit = _resolveComponent("q-popup-edit")

              return (_openBlock(), _createBlock(_resolveDynamicComponent(_ctx.labelType), {
                id: _ctx.id,
                class: _normalizeClass(["editable-label cursor-pointer", {'m-editable-empty': _ctx.isEmpty}]),
                title: _ctx.tooltip
              }, {
                default: _withCtx(() => [
                  (_ctx.isEmpty)
                    ? (_openBlock(), _createElementBlock("span", {
                        key: 0,
                        class: "m-muted m-editable-placeholder",
                        textContent: _toDisplayString(_ctx.placeholder || _ctx.tooltip)
                      }, null, 8, ["textContent"]))
                    : (_openBlock(), _createElementBlock("span", {
                        key: 1,
                        innerHTML: _ctx.shown
                      }, null, 8, ["innerHTML"])),
                  _createVNode(_component_q_popup_edit, {
                    modelValue: _ctx.editValue,
                    "onUpdate:modelValue": $event => ((_ctx.editValue) = $event),
                    buttons: "",
                    "label-set": _ctx.submit,
                    "label-cancel": _ctx.cancel,
                    onBeforeShow: _ctx.beforeShow,
                    onSave: _ctx.save
                  }, {
                    default: _withCtx((scope) => [
                      (_ctx.widgetType === 'select')
                        ? (_openBlock(), _createBlock(_component_q_select, {
                            key: 0,
                            modelValue: scope.value,
                            "onUpdate:modelValue": $event => ((scope.value) = $event),
                            options: _ctx.selectOptions,
                            "emit-value": "",
                            "map-options": "",
                            dense: "",
                            outlined: "",
                            "options-dense": "",
                            autofocus: "",
                            loading: _ctx.saving || _ctx.loadingOptions,
                            class: "m-editable-select"
                          }, null, 8, ["modelValue", "onUpdate:modelValue", "options", "loading"]))
                        : (_ctx.isDate)
                          ? (_openBlock(), _createElementBlock("div", {
                              key: 1,
                              class: "row no-wrap q-gutter-sm m-editable-date"
                            }, [
                              _createVNode(_component_q_date, {
                                modelValue: scope.value,
                                "onUpdate:modelValue": $event => ((scope.value) = $event),
                                mask: _ctx.dateMask,
                                minimal: "",
                                flat: ""
                              }, null, 8, ["modelValue", "onUpdate:modelValue", "mask"]),
                              (_ctx.widgetType === 'date-time')
                                ? (_openBlock(), _createBlock(_component_q_time, {
                                    key: 0,
                                    modelValue: scope.value,
                                    "onUpdate:modelValue": $event => ((scope.value) = $event),
                                    mask: _ctx.dateMask,
                                    format24h: "",
                                    flat: ""
                                  }, null, 8, ["modelValue", "onUpdate:modelValue", "mask"]))
                                : _createCommentVNode("", true)
                            ]))
                          : (_openBlock(), _createBlock(_component_q_input, {
                              key: 2,
                              modelValue: scope.value,
                              "onUpdate:modelValue": $event => ((scope.value) = $event),
                              dense: "",
                              autofocus: "",
                              type: _ctx.widgetType === 'textarea' ? 'textarea' : 'text',
                              loading: _ctx.saving,
                              onKeyup: _withKeys($event => (_ctx.widgetType === 'textarea' ? null : scope.set()), ["enter"])
                            }, null, 8, ["modelValue", "onUpdate:modelValue", "type", "loading", "onKeyup"]))
                    ]),
                    _: 1
                  }, 8, ["modelValue", "onUpdate:modelValue", "label-set", "label-cancel", "onBeforeShow", "onSave"])
                ]),
                _: 1
              }, 8, ["id", "class", "title"]))
            }
        } },
        "m-field": { hash: "15d75803", make: function (Vue) {
            const { renderSlot: _renderSlot, resolveComponent: _resolveComponent, mergeProps: _mergeProps, withCtx: _withCtx, openBlock: _openBlock, createBlock: _createBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_field = _resolveComponent("q-field")

              return (_openBlock(), _createBlock(_component_q_field, _mergeProps({
                dense: "",
                outlined: !_ctx.borderless,
                borderless: _ctx.borderless,
                "stack-label": "",
                "hide-bottom-space": "",
                label: _ctx.label,
                "bg-color": _ctx.bgColor,
                disable: _ctx.disable
              }, _ctx.$attrs, { "model-value": 1 }), {
                control: _withCtx(() => [
                  _renderSlot(_ctx.$slots, "default")
                ]),
                _: 3
              }, 16, ["outlined", "borderless", "label", "bg-color", "disable"]))
            }
        } },
        "m-file": { hash: "4423bd8", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, withCtx: _withCtx, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, mergeProps: _mergeProps } = Vue

            return function render(_ctx, _cache) {
              const _component_q_icon = _resolveComponent("q-icon")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_file = _resolveComponent("q-file")

              return (_openBlock(), _createBlock(_component_q_file, _mergeProps({
                dense: "",
                outlined: "",
                "stack-label": "",
                "hide-bottom-space": "",
                "no-error-icon": "",
                "use-chips": _ctx.multiple,
                counter: _ctx.multiple,
                label: _ctx.label,
                "model-value": _ctx.modelValue,
                "onUpdate:modelValue": $event => (_ctx.$emit('update:modelValue', $event)),
                name: _ctx.name,
                for: _ctx.id,
                size: _ctx.size,
                multiple: _ctx.multiple,
                accept: _ctx.accept,
                form: _ctx.form,
                disable: _ctx.disable,
                "bg-color": _ctx.bg
              }, _ctx.errorAttrs), {
                append: _withCtx(() => [
                  _createVNode(_component_q_icon, {
                    name: "attach_file",
                    size: "xs"
                  })
                ]),
                default: _withCtx(() => [
                  (_ctx.tooltip)
                    ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                        ]),
                        _: 1
                      }))
                    : _createCommentVNode("", true)
                ]),
                _: 1
              }, 16, ["use-chips", "counter", "label", "model-value", "onUpdate:modelValue", "name", "for", "size", "multiple", "accept", "form", "disable", "bg-color"]))
            }
        } },
        "m-form": { hash: "ffcdd952", make: function (Vue) {
            const { renderSlot: _renderSlot, resolveComponent: _resolveComponent, withModifiers: _withModifiers, withCtx: _withCtx, openBlock: _openBlock, createBlock: _createBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_form = _resolveComponent("q-form")

              return (_openBlock(), _createBlock(_component_q_form, {
                ref: "qForm",
                onSubmit: _withModifiers(_ctx.submitForm, ["prevent"]),
                onReset: _withModifiers(_ctx.resetForm, ["prevent"]),
                autocapitalize: "off",
                autocomplete: "off"
              }, {
                default: _withCtx(() => [
                  _renderSlot(_ctx.$slots, "default", {
                    fields: _ctx.fields,
                    checkboxAllState: _ctx.checkboxAllState,
                    setCheckboxAllState: _ctx.setCheckboxAllState,
                    checkboxStates: _ctx.checkboxStates,
                    clickCheckbox: _ctx.clickCheckbox,
                    addCheckboxParameters: _ctx.addCheckboxParameters,
                    blurSubmitForm: _ctx.blurSubmitForm,
                    hasFieldsChanged: _ctx.hasFieldsChanged,
                    fieldChanged: _ctx.fieldChanged
                  })
                ]),
                _: 3
              }, 8, ["onSubmit", "onReset"]))
            }
        } },
        "m-form-column-config": { hash: "ab938ef1", make: function (Vue) {
            const { renderList: _renderList, Fragment: _Fragment, openBlock: _openBlock, createElementBlock: _createElementBlock, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, createVNode: _createVNode, createBlock: _createBlock, createCommentVNode: _createCommentVNode, resolveDirective: _resolveDirective, withDirectives: _withDirectives, withModifiers: _withModifiers, createElementVNode: _createElementVNode } = Vue

            return function render(_ctx, _cache) {
              const _component_q_item_label = _resolveComponent("q-item-label")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_item_section = _resolveComponent("q-item-section")
              const _component_q_item = _resolveComponent("q-item")
              const _component_q_list = _resolveComponent("q-list")
              const _component_q_btn_dropdown = _resolveComponent("q-btn-dropdown")
              const _component_q_btn_group = _resolveComponent("q-btn-group")
              const _component_m_form = _resolveComponent("m-form")
              const _directive_close_popup = _resolveDirective("close-popup")

              return (_openBlock(), _createBlock(_component_m_form, {
                ref: "mForm",
                id: _ctx.id,
                action: _ctx.action
              }, {
                default: _withCtx(() => [
                  (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.columns, (column, columnIdx) => {
                    return (_openBlock(), _createBlock(_component_q_list, {
                      key: column.id,
                      bordered: "",
                      dense: "",
                      class: "q-mb-sm rounded-borders"
                    }, {
                      default: _withCtx(() => [
                        _createVNode(_component_q_item_label, { header: "" }, {
                          default: _withCtx(() => [
                            _createTextVNode(_toDisplayString(column.label), 1)
                          ]),
                          _: 2
                        }, 1024),
                        (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(column.children, (field, fieldIdx) => {
                          return (_openBlock(), _createBlock(_component_q_item, {
                            key: field.id
                          }, {
                            default: _withCtx(() => [
                              (columnIdx !== 0)
                                ? (_openBlock(), _createBlock(_component_q_item_section, {
                                    key: 0,
                                    side: ""
                                  }, {
                                    default: _withCtx(() => [
                                      _createVNode(_component_q_btn, {
                                        flat: "",
                                        round: "",
                                        dense: "",
                                        size: "sm",
                                        icon: "close",
                                        onClick: $event => (_ctx.hideField(columnIdx, fieldIdx))
                                      }, {
                                        default: _withCtx(() => [
                                          _createVNode(_component_q_tooltip, null, {
                                            default: _withCtx(() => [
                                              _createTextVNode("Hide")
                                            ]),
                                            _: 1
                                          })
                                        ]),
                                        _: 1
                                      }, 8, ["onClick"])
                                    ]),
                                    _: 2
                                  }, 1024))
                                : _createCommentVNode("", true),
                              _createVNode(_component_q_item_section, null, {
                                default: _withCtx(() => [
                                  _createVNode(_component_q_item_label, null, {
                                    default: _withCtx(() => [
                                      _createTextVNode(_toDisplayString(field.label), 1)
                                    ]),
                                    _: 2
                                  }, 1024)
                                ]),
                                _: 2
                              }, 1024),
                              (columnIdx === 0)
                                ? (_openBlock(), _createBlock(_component_q_item_section, {
                                    key: 1,
                                    side: ""
                                  }, {
                                    default: _withCtx(() => [
                                      _createVNode(_component_q_btn_dropdown, {
                                        dense: "",
                                        flat: "",
                                        "no-caps": "",
                                        label: "Display",
                                        class: "m-btn-tonal"
                                      }, {
                                        default: _withCtx(() => [
                                          _createVNode(_component_q_list, { dense: "" }, {
                                            default: _withCtx(() => [
                                              (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.columns.slice(1), (toColumn, toColumnIdx) => {
                                                return _withDirectives((_openBlock(), _createBlock(_component_q_item, {
                                                  key: toColumn.id,
                                                  clickable: ""
                                                }, {
                                                  default: _withCtx(() => [
                                                    _createVNode(_component_q_item_section, {
                                                      onClick: $event => (_ctx.moveToCol(columnIdx, fieldIdx, toColumnIdx+1))
                                                    }, {
                                                      default: _withCtx(() => [
                                                        _createTextVNode(_toDisplayString(toColumn.label), 1)
                                                      ]),
                                                      _: 2
                                                    }, 1032, ["onClick"])
                                                  ]),
                                                  _: 2
                                                }, 1024)), [
                                                  [_directive_close_popup]
                                                ])
                                              }), 128)),
                                              _withDirectives((_openBlock(), _createBlock(_component_q_item, { clickable: "" }, {
                                                default: _withCtx(() => [
                                                  _createVNode(_component_q_item_section, {
                                                    onClick: $event => (_ctx.moveToCol(columnIdx, fieldIdx, _ctx.columns.length+1))
                                                  }, {
                                                    default: _withCtx(() => [
                                                      _createTextVNode("New Column")
                                                    ]),
                                                    _: 1
                                                  }, 8, ["onClick"])
                                                ]),
                                                _: 2
                                              }, 1024)), [
                                                [_directive_close_popup]
                                              ])
                                            ]),
                                            _: 2
                                          }, 1024)
                                        ]),
                                        _: 2
                                      }, 1024)
                                    ]),
                                    _: 2
                                  }, 1024))
                                : (_openBlock(), _createBlock(_component_q_item_section, {
                                    key: 2,
                                    side: ""
                                  }, {
                                    default: _withCtx(() => [
                                      _createVNode(_component_q_btn_group, { flat: "" }, {
                                        default: _withCtx(() => [
                                          _createVNode(_component_q_btn, {
                                            disabled: columnIdx <= 1,
                                            dense: "",
                                            flat: "",
                                            size: "sm",
                                            icon: "north",
                                            onClick: $event => (_ctx.moveToCol(columnIdx, fieldIdx, columnIdx-1))
                                          }, null, 8, ["disabled", "onClick"]),
                                          _createVNode(_component_q_btn, {
                                            disabled: fieldIdx === 0,
                                            dense: "",
                                            flat: "",
                                            size: "sm",
                                            icon: "expand_less",
                                            onClick: $event => (_ctx.moveInCol(columnIdx, fieldIdx, fieldIdx-1))
                                          }, null, 8, ["disabled", "onClick"]),
                                          _createVNode(_component_q_btn, {
                                            disabled: (fieldIdx + 1) === column.children.length,
                                            dense: "",
                                            flat: "",
                                            size: "sm",
                                            icon: "expand_more",
                                            onClick: $event => (_ctx.moveInCol(columnIdx, fieldIdx, fieldIdx+1))
                                          }, null, 8, ["disabled", "onClick"]),
                                          _createVNode(_component_q_btn, {
                                            dense: "",
                                            flat: "",
                                            size: "sm",
                                            icon: "south",
                                            onClick: $event => (_ctx.moveToCol(columnIdx, fieldIdx, columnIdx+1))
                                          }, null, 8, ["onClick"])
                                        ]),
                                        _: 2
                                      }, 1024)
                                    ]),
                                    _: 2
                                  }, 1024))
                            ]),
                            _: 2
                          }, 1024))
                        }), 128))
                      ]),
                      _: 2
                    }, 1024))
                  }), 128)),
                  _createElementVNode("div", { class: "q-my-md" }, [
                    _createVNode(_component_q_btn, {
                      dense: "",
                      unelevated: "",
                      "no-caps": "",
                      color: "primary",
                      onClick: _withModifiers($event => (_ctx.saveColumns()), ["prevent"]),
                      label: "Save Changes"
                    }, null, 8, ["onClick"]),
                    _createVNode(_component_q_btn, {
                      dense: "",
                      flat: "",
                      "no-caps": "",
                      onClick: _withModifiers($event => (_ctx.resetColumns()), ["prevent"]),
                      label: "Undo Changes"
                    }, null, 8, ["onClick"]),
                    _createVNode(_component_q_btn, {
                      dense: "",
                      flat: "",
                      "no-caps": "",
                      onClick: _withModifiers($event => (_ctx.resetToDefault()), ["prevent"]),
                      label: "Reset to Default"
                    }, null, 8, ["onClick"])
                  ])
                ]),
                _: 1
              }, 8, ["id", "action"]))
            }
        } },
        "m-form-go-page": { hash: "1b62080b", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, withCtx: _withCtx, withModifiers: _withModifiers, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode } = Vue

            return function render(_ctx, _cache) {
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_input = _resolveComponent("q-input")
              const _component_q_form = _resolveComponent("q-form")

              return (!_ctx.formList || (_ctx.formList.paginate && _ctx.formList.paginate.pageMaxIndex > 4))
                ? (_openBlock(), _createBlock(_component_q_form, {
                    key: 0,
                    onSubmit: _withModifiers(_ctx.goPage, ["prevent"]),
                    class: "q-mx-xs"
                  }, {
                    default: _withCtx(() => [
                      _createVNode(_component_q_input, {
                        dense: "",
                        outlined: "",
                        "hide-bottom-space": "",
                        modelValue: _ctx.pageIndex,
                        "onUpdate:modelValue": $event => ((_ctx.pageIndex) = $event),
                        type: "text",
                        name: "pageIndex",
                        placeholder: "Page #",
                        style: {"width":"96px"},
                        "input-class": "text-center",
                        rules: [val => /^\d*$/.test(val) || 'digits only', val => !val || ((_ctx.formList && +val <= _ctx.formList.paginate.pageMaxIndex + 1) || (_ctx.maxIndex && +val <= _ctx.maxIndex + 1)) || 'higher than max']
                      }, {
                        append: _withCtx(() => [
                          _createVNode(_component_q_btn, {
                            flat: "",
                            round: "",
                            dense: "",
                            size: "sm",
                            type: "submit",
                            icon: "arrow_forward"
                          })
                        ]),
                        _: 1
                      }, 8, ["modelValue", "onUpdate:modelValue", "rules"])
                    ]),
                    _: 1
                  }, 8, ["onSubmit"]))
                : _createCommentVNode("", true)
            }
        } },
        "m-form-link": { hash: "815e6a35", make: function (Vue) {
            const { renderSlot: _renderSlot, resolveComponent: _resolveComponent, withModifiers: _withModifiers, withCtx: _withCtx, openBlock: _openBlock, createBlock: _createBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_form = _resolveComponent("q-form")

              return (_openBlock(), _createBlock(_component_q_form, {
                ref: "qForm",
                onSubmit: _withModifiers(_ctx.submitForm, ["prevent"]),
                onReset: _withModifiers(_ctx.resetForm, ["prevent"]),
                autocapitalize: "off",
                autocomplete: "off"
              }, {
                default: _withCtx(() => [
                  _renderSlot(_ctx.$slots, "default", {
                    clearForm: _ctx.clearForm,
                    fields: _ctx.fields,
                    hasFieldsChanged: _ctx.hasFieldsChanged,
                    fieldChanged: _ctx.fieldChanged
                  })
                ]),
                _: 3
              }, 8, ["onSubmit", "onReset"]))
            }
        } },
        "m-form-list": { hash: "ae8730d0", make: function (Vue) {
            const { renderList: _renderList, Fragment: _Fragment, openBlock: _openBlock, createElementBlock: _createElementBlock, renderSlot: _renderSlot, resolveComponent: _resolveComponent, withCtx: _withCtx, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createElementVNode: _createElementVNode, createVNode: _createVNode, normalizeClass: _normalizeClass } = Vue

            return function render(_ctx, _cache) {
              const _component_m_form = _resolveComponent("m-form")
              const _component_q_space = _resolveComponent("q-space")
              const _component_m_form_paginate = _resolveComponent("m-form-paginate")
              const _component_m_form_go_page = _resolveComponent("m-form-go-page")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_m_form_link = _resolveComponent("m-form-link")
              const _component_q_checkbox = _resolveComponent("q-checkbox")

              return (_openBlock(), _createElementBlock("div", null, [
                (!_ctx.multi && !_ctx.skipForm)
                  ? (_openBlock(true), _createElementBlock(_Fragment, { key: 0 }, _renderList(_ctx.rowList, (fields, rowIndex) => {
                      return (_openBlock(), _createBlock(_component_m_form, {
                        key: rowIndex,
                        name: _ctx.idVal+'_'+rowIndex,
                        id: _ctx.idVal+'_'+rowIndex,
                        action: _ctx.action,
                        "fields-initial": fields
                      }, {
                        default: _withCtx(() => [
                          _renderSlot(_ctx.$slots, "rowForm", {
                            fields: fields,
                            rowIndex: rowIndex
                          })
                        ]),
                        _: 2
                      }, 1032, ["name", "id", "action", "fields-initial"]))
                    }), 128))
                  : _createCommentVNode("", true),
                (_ctx.multi && !_ctx.skipForm)
                  ? (_openBlock(), _createBlock(_component_m_form, {
                      key: 1,
                      name: _ctx.idVal,
                      id: _ctx.idVal,
                      action: _ctx.action,
                      "fields-initial": _ctx.multiFields
                    }, {
                      default: _withCtx(() => [
                        _createElementVNode("input", {
                          type: "hidden",
                          name: "moquiFormName",
                          value: _ctx.name
                        }, null, 8, ["value"]),
                        _createElementVNode("input", {
                          type: "hidden",
                          name: "_isMulti",
                          value: "true"
                        }),
                        (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.rowList, (fields, rowIndex) => {
                          return _renderSlot(_ctx.$slots, "rowForm", {
                            fields: fields,
                            rowIndex: rowIndex
                          }, undefined, undefined, rowIndex)
                        }), 128))
                      ]),
                      _: 3
                    }, 8, ["name", "id", "action", "fields-initial"]))
                  : _createCommentVNode("", true),
                _createElementVNode("div", {
                  class: _normalizeClass(["q-my-sm q-table__container q-table__card q-table--horizontal-separator q-table--dense q-table--flat", {'q-table--dark':_ctx.$q.dark.isActive}])
                }, [
                  _createElementVNode("div", { class: "form-list-nav-row row items-center" }, [
                    _renderSlot(_ctx.$slots, "navStart"),
                    _createVNode(_component_q_space),
                    _createVNode(_component_m_form_paginate, {
                      paginate: _ctx.paginate,
                      "form-list": this
                    }, null, 8, ["paginate"]),
                    _createVNode(_component_m_form_go_page, {
                      "id-val": _ctx.idVal,
                      "form-list": this
                    }, null, 8, ["id-val"]),
                    (_ctx.csvButton)
                      ? (_openBlock(), _createBlock(_component_q_btn, {
                          key: 0,
                          dense: "",
                          flat: "",
                          "no-caps": "",
                          icon: "download",
                          type: "a",
                          href: _ctx.csvUrl,
                          label: "CSV"
                        }, null, 8, ["href"]))
                      : _createCommentVNode("", true),
                    _renderSlot(_ctx.$slots, "nav")
                  ]),
                  _createElementVNode("div", {
                    class: "table q-table",
                    id: _ctx.idVal+'_table'
                  }, [
                    (!_ctx.skipHeader)
                      ? (_openBlock(), _createElementBlock("div", {
                          key: 0,
                          class: "thead"
                        }, [
                          (_ctx.headerForm && !_ctx.headerDialog)
                            ? (_openBlock(), _createBlock(_component_m_form_link, {
                                key: 0,
                                name: _ctx.idVal+'_header',
                                id: _ctx.idVal+'_header',
                                action: _ctx.$root.currentLinkPath,
                                "fields-initial": _ctx.searchObj
                              }, {
                                default: _withCtx((formProps) => [
                                  _renderSlot(_ctx.$slots, "headerForm", {
                                    search: formProps.fields
                                  })
                                ]),
                                _: 3
                              }, 8, ["name", "id", "action", "fields-initial"]))
                            : _createCommentVNode("", true),
                          _createElementVNode("div", { class: "tr" }, [
                            (_ctx.selection)
                              ? (_openBlock(), _createElementBlock("div", {
                                  key: 0,
                                  class: "th m-sel"
                                }, [
                                  _createVNode(_component_q_checkbox, {
                                    dense: "",
                                    size: "sm",
                                    "model-value": _ctx.allSelected,
                                    "onUpdate:modelValue": _ctx.selectAll
                                  }, null, 8, ["model-value", "onUpdate:modelValue"])
                                ]))
                              : _createCommentVNode("", true),
                            _renderSlot(_ctx.$slots, "header", {
                              search: _ctx.searchObj,
                              sortBy: _ctx.sortBy,
                              orderBy: _ctx.orderBy
                            })
                          ])
                        ]))
                      : _createCommentVNode("", true),
                    _createElementVNode("div", { class: "tbody" }, [
                      (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.rowList, (fields, rowIndex) => {
                        return (_openBlock(), _createElementBlock(_Fragment, { key: rowIndex }, [
                          _createElementVNode("div", {
                            class: _normalizeClass(["tr", {selected:_ctx.selected[rowIndex]}])
                          }, [
                            (_ctx.selection)
                              ? (_openBlock(), _createElementBlock("div", {
                                  key: 0,
                                  class: "td m-sel"
                                }, [
                                  _createVNode(_component_q_checkbox, {
                                    dense: "",
                                    size: "sm",
                                    "model-value": !!_ctx.selected[rowIndex],
                                    "onUpdate:modelValue": $event => (_ctx.selected[rowIndex] = $event)
                                  }, null, 8, ["model-value", "onUpdate:modelValue"])
                                ]))
                              : _createCommentVNode("", true),
                            _renderSlot(_ctx.$slots, "row", {
                              fields: fields,
                              rowIndex: rowIndex,
                              selected: !!_ctx.selected[rowIndex]
                            })
                          ], 2),
                          (fields.aggregateSubList && fields.aggregateSubList.length)
                            ? (_openBlock(true), _createElementBlock(_Fragment, { key: 0 }, _renderList(fields.aggregateSubList, (subRow, subIndex) => {
                                return (_openBlock(), _createElementBlock("div", {
                                  key: rowIndex+'_'+subIndex,
                                  class: "tr form-list-sub-row"
                                }, [
                                  _renderSlot(_ctx.$slots, "subRow", {
                                    row: fields,
                                    subRow: subRow,
                                    subIndex: subIndex
                                  })
                                ]))
                              }), 128))
                            : _createCommentVNode("", true)
                        ], 64))
                      }), 128)),
                      (!_ctx.rowList.length && !_ctx.loading)
                        ? (_openBlock(), _createElementBlock("div", {
                            key: 0,
                            class: "tr"
                          }, [
                            _createElementVNode("div", { class: "td" }, "No results found")
                          ]))
                        : _createCommentVNode("", true)
                    ])
                  ], 8, ["id"]),
                  (_ctx.paginate && _ctx.paginate.pageMaxIndex > 0)
                    ? (_openBlock(), _createElementBlock("div", {
                        key: 0,
                        class: "form-list-nav-row row items-center"
                      }, [
                        _createVNode(_component_q_space),
                        _createVNode(_component_m_form_go_page, {
                          "id-val": _ctx.idVal,
                          "form-list": this
                        }, null, 8, ["id-val"]),
                        _createVNode(_component_m_form_paginate, {
                          paginate: _ctx.paginate,
                          "form-list": this
                        }, null, 8, ["paginate"])
                      ]))
                    : _createCommentVNode("", true)
                ], 2)
              ]))
            }
        } },
        "m-form-paginate": { hash: "e892c70f", make: function (Vue) {
            const { toDisplayString: _toDisplayString, createElementVNode: _createElementVNode, resolveComponent: _resolveComponent, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_pagination = _resolveComponent("q-pagination")

              return (_ctx.paginate && _ctx.paginate.count > 1)
                ? (_openBlock(), _createElementBlock("div", {
                    key: 0,
                    class: "row no-wrap items-center"
                  }, [
                    _createElementVNode("span", { class: "text-caption text-grey-7 q-mx-sm" }, _toDisplayString(_ctx.paginate.pageRangeLow) + "-" + _toDisplayString(_ctx.paginate.pageRangeHigh) + " / " + _toDisplayString(_ctx.paginate.count), 1),
                    (_ctx.paginate.pageMaxIndex > 0)
                      ? (_openBlock(), _createBlock(_component_q_pagination, {
                          key: 0,
                          "model-value": _ctx.paginate.pageIndex + 1,
                          "onUpdate:modelValue": $event => (_ctx.setIndex($event - 1)),
                          max: _ctx.paginate.pageMaxIndex + 1,
                          "max-pages": 7,
                          "direction-links": "",
                          "boundary-links": "",
                          size: "sm",
                          flat: "",
                          "active-design": "unelevated",
                          color: "grey-8",
                          "active-color": "primary"
                        }, null, 8, ["model-value", "onUpdate:modelValue", "max"]))
                      : _createCommentVNode("", true)
                  ]))
                : _createCommentVNode("", true)
            }
        } },
        "m-html-view": { hash: "b2da5b44", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, toDisplayString: _toDisplayString, createElementVNode: _createElementVNode, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, normalizeClass: _normalizeClass, normalizeStyle: _normalizeStyle } = Vue

            return function render(_ctx, _cache) {
              const _component_q_icon = _resolveComponent("q-icon")
              const _component_q_btn = _resolveComponent("q-btn")

              return (_openBlock(), _createElementBlock("div", {
                class: "m-html-view",
                "data-state": _ctx.state,
                "data-blocked": _ctx.blocked
              }, [
                (_ctx.blocked && !_ctx.remoteAllowed && _ctx.imagesBar)
                  ? (_openBlock(), _createElementBlock("div", {
                      key: 0,
                      class: "m-html-view-images row items-center no-wrap"
                    }, [
                      _createVNode(_component_q_icon, {
                        name: "sym_o_hide_image",
                        size: "xs",
                        class: "q-mr-sm"
                      }),
                      _createElementVNode("span", { class: "col m-muted" }, _toDisplayString(_ctx.t("Images from this sender are hidden")), 1),
                      _createVNode(_component_q_btn, {
                        flat: "",
                        dense: "",
                        "no-caps": "",
                        color: "primary",
                        class: "m-html-view-show",
                        label: _ctx.t('Show images'),
                        onClick: _ctx.showImages
                      }, null, 8, ["label", "onClick"])
                    ]))
                  : _createCommentVNode("", true),
                _createElementVNode("iframe", {
                  ref: "frame",
                  class: _normalizeClass(["m-html-view-frame", {'m-html-view-card': _ctx.designed && _ctx.isDark}]),
                  title: _ctx.title,
                  sandbox: "allow-same-origin allow-popups allow-popups-to-escape-sandbox",
                  referrerpolicy: "no-referrer",
                  srcdoc: _ctx.srcdoc,
                  style: _normalizeStyle(_ctx.frameStyle),
                  onLoad: _ctx.onLoad
                }, null, 46, ["title", "srcdoc", "onLoad"])
              ], 8, ["data-state", "data-blocked"]))
            }
        } },
        "m-link": { hash: "449c1400", make: function (Vue) {
            const { renderSlot: _renderSlot, openBlock: _openBlock, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createElementBlock("a", {
                href: _ctx.linkHref,
                onClick: _ctx.go,
                class: "q-link"
              }, [
                _renderSlot(_ctx.$slots, "default")
              ], 8, ["href", "onClick"]))
            }
        } },
        "m-list-views": { hash: "f0f56d4d", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, withCtx: _withCtx, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, renderSlot: _renderSlot, createElementVNode: _createElementVNode, renderList: _renderList, Fragment: _Fragment, normalizeClass: _normalizeClass, createBlock: _createBlock, withModifiers: _withModifiers, normalizeStyle: _normalizeStyle } = Vue

            return function render(_ctx, _cache) {
              const _component_q_space = _resolveComponent("q-space")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_btn_toggle = _resolveComponent("q-btn-toggle")
              const _component_q_badge = _resolveComponent("q-badge")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_card = _resolveComponent("q-card")
              const _component_q_separator = _resolveComponent("q-separator")
              const _component_q_item_section = _resolveComponent("q-item-section")
              const _component_q_item_label = _resolveComponent("q-item-label")
              const _component_q_item = _resolveComponent("q-item")
              const _component_q_list = _resolveComponent("q-list")
              const _component_q_menu = _resolveComponent("q-menu")
              const _component_m_dynamic_container = _resolveComponent("m-dynamic-container")
              const _component_m_text_line = _resolveComponent("m-text-line")
              const _component_m_date_time = _resolveComponent("m-date-time")
              const _component_m_form = _resolveComponent("m-form")
              const _component_m_dialog = _resolveComponent("m-dialog")

              return (_openBlock(), _createElementBlock("div", {
                class: _normalizeClass(["m-list-views", 'm-list-views--' + _ctx.current]),
                id: _ctx.id,
                "data-view": _ctx.current
              }, [
                (_ctx.viewOptions.length > 1)
                  ? (_openBlock(), _createElementBlock("div", {
                      key: 0,
                      class: "m-list-views-bar row items-center no-wrap"
                    }, [
                      _createVNode(_component_q_space),
                      _createVNode(_component_q_btn_toggle, {
                        "model-value": _ctx.current,
                        "onUpdate:modelValue": _ctx.setView,
                        dense: "",
                        unelevated: "",
                        "no-caps": "",
                        "toggle-color": "primary",
                        class: "m-list-views-toggle",
                        options: _ctx.viewOptions
                      }, {
                        list: _withCtx(() => [
                          _createVNode(_component_q_tooltip, null, {
                            default: _withCtx(() => [
                              _createTextVNode(_toDisplayString(_ctx.labels.list), 1)
                            ]),
                            _: 1
                          })
                        ]),
                        board: _withCtx(() => [
                          _createVNode(_component_q_tooltip, null, {
                            default: _withCtx(() => [
                              _createTextVNode(_toDisplayString(_ctx.labels.board), 1)
                            ]),
                            _: 1
                          })
                        ]),
                        calendar: _withCtx(() => [
                          _createVNode(_component_q_tooltip, null, {
                            default: _withCtx(() => [
                              _createTextVNode(_toDisplayString(_ctx.labels.calendar), 1)
                            ]),
                            _: 1
                          })
                        ]),
                        _: 1
                      }, 8, ["model-value", "onUpdate:modelValue", "options"])
                    ]))
                  : _createCommentVNode("", true),
                _createElementVNode("div", { class: "m-list-views-list" }, [
                  _renderSlot(_ctx.$slots, "default")
                ]),
                (_ctx.current === 'board')
                  ? (_openBlock(), _createElementBlock("div", {
                      key: 1,
                      ref: "board",
                      class: _normalizeClass(["m-board row no-wrap", {'m-board--dragging': !!_ctx.dragging}])
                    }, [
                      (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.columns, (column) => {
                        return (_openBlock(), _createElementBlock("div", {
                          key: column.value,
                          class: _normalizeClass(["m-board-col column no-wrap", _ctx.dragging ? (column.value === _ctx.dragging.from ? 'm-board-col--from' : (_ctx.dragAllowed(column.value) ? 'm-board-col--target' : 'm-board-col--blocked')) : '']),
                          "data-value": column.value
                        }, [
                          _createElementVNode("div", { class: "m-board-col-head row items-center no-wrap" }, [
                            _createElementVNode("span", {
                              class: _normalizeClass(["m-board-col-dot", 'bg-' + (column.color || 'grey-6')])
                            }, null, 2),
                            _createElementVNode("span", { class: "m-board-col-label ellipsis" }, _toDisplayString(column.label), 1),
                            _createVNode(_component_q_badge, {
                              class: _normalizeClass(["m-board-col-count m-tonal q-ml-xs", column.color ? 'text-' + column.color : 'text-grey-8'])
                            }, {
                              default: _withCtx(() => [
                                _createTextVNode(_toDisplayString(column.count), 1)
                              ]),
                              _: 2
                            }, 1032, ["class"]),
                            _createVNode(_component_q_space),
                            (_ctx.board.sumField)
                              ? (_openBlock(), _createElementBlock("span", {
                                  key: 0,
                                  class: "m-board-col-sum m-muted"
                                }, _toDisplayString(_ctx.sumText(column.sum)), 1))
                              : _createCommentVNode("", true)
                          ]),
                          _createElementVNode("div", {
                            class: "m-board-cards col",
                            "data-value": column.value
                          }, [
                            (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(column.items, (item) => {
                              return (_openBlock(), _createElementBlock("div", {
                                key: item.index,
                                class: "m-board-card",
                                "data-index": item.index,
                                onClick: $event => (_ctx.openCard($event))
                              }, [
                                _renderSlot(_ctx.$slots, 'card_' + item.index, {
                                  index: item.index,
                                  row: item.row
                                }, () => [
                                  _renderSlot(_ctx.$slots, "card", {
                                    index: item.index,
                                    row: item.row
                                  }, () => [
                                    _createElementVNode("div", { class: "m-card-title" }, _toDisplayString(_ctx.cardTitle(item.row)), 1)
                                  ])
                                ])
                              ], 8, ["data-index", "onClick"]))
                            }), 128))
                          ], 8, ["data-value"]),
                          (!column.count)
                            ? (_openBlock(), _createElementBlock("div", {
                                key: 0,
                                class: "m-board-empty m-muted"
                              }, _toDisplayString(_ctx.labels.noCards), 1))
                            : _createCommentVNode("", true)
                        ], 10, ["data-value"]))
                      }), 128)),
                      (!_ctx.columns.length)
                        ? (_openBlock(), _createElementBlock("div", {
                            key: 0,
                            class: "m-board-empty m-muted q-pa-md"
                          }, _toDisplayString(_ctx.labels.noCards), 1))
                        : _createCommentVNode("", true)
                    ], 2))
                  : _createCommentVNode("", true),
                (_ctx.current === 'calendar')
                  ? (_openBlock(), _createElementBlock("div", {
                      key: 2,
                      class: _normalizeClass(["m-calendar", {'m-calendar--dragging': !!_ctx.drag && _ctx.drag.moved}])
                    }, [
                      _createElementVNode("div", { class: "m-calendar-bar row items-center no-wrap" }, [
                        _createVNode(_component_q_btn, {
                          unelevated: "",
                          dense: "",
                          "no-caps": "",
                          class: "m-btn-tonal q-px-sm",
                          label: _ctx.labels.today,
                          onClick: _ctx.goToday
                        }, null, 8, ["label", "onClick"]),
                        (_ctx.calMode !== 'list')
                          ? (_openBlock(), _createElementBlock(_Fragment, { key: 0 }, [
                              _createVNode(_component_q_btn, {
                                flat: "",
                                dense: "",
                                round: "",
                                icon: "chevron_left",
                                class: "m-cal-prev",
                                onClick: $event => (_ctx.shift(-1))
                              }, {
                                default: _withCtx(() => [
                                  _createVNode(_component_q_tooltip, null, {
                                    default: _withCtx(() => [
                                      _createTextVNode(_toDisplayString(_ctx.labels.previous), 1)
                                    ]),
                                    _: 1
                                  })
                                ]),
                                _: 1
                              }, 8, ["onClick"]),
                              _createVNode(_component_q_btn, {
                                flat: "",
                                dense: "",
                                round: "",
                                icon: "chevron_right",
                                class: "m-cal-next",
                                onClick: $event => (_ctx.shift(1))
                              }, {
                                default: _withCtx(() => [
                                  _createVNode(_component_q_tooltip, null, {
                                    default: _withCtx(() => [
                                      _createTextVNode(_toDisplayString(_ctx.labels.next), 1)
                                    ]),
                                    _: 1
                                  })
                                ]),
                                _: 1
                              }, 8, ["onClick"])
                            ], 64))
                          : _createCommentVNode("", true),
                        _createElementVNode("div", {
                          class: _normalizeClass(["m-calendar-title ellipsis q-ml-sm", _ctx.phone ? 'text-subtitle1 text-weight-medium' : 'text-h6'])
                        }, _toDisplayString(_ctx.rangeTitle), 3),
                        _createVNode(_component_q_space),
                        (_ctx.modeOptions.length > 1)
                          ? (_openBlock(), _createBlock(_component_q_btn_toggle, {
                              key: 1,
                              "model-value": _ctx.calMode,
                              "onUpdate:modelValue": _ctx.setMode,
                              dense: "",
                              "no-caps": "",
                              unelevated: "",
                              "toggle-color": "primary",
                              class: "m-cal-modes",
                              options: _ctx.modeOptions
                            }, null, 8, ["model-value", "onUpdate:modelValue", "options"]))
                          : _createCommentVNode("", true)
                      ]),
                      (_ctx.calMode === 'month')
                        ? (_openBlock(), _createBlock(_component_q_card, {
                            key: 0,
                            flat: "",
                            bordered: "",
                            class: "m-cal-month"
                          }, {
                            default: _withCtx(() => [
                              _createElementVNode("div", { class: "row no-wrap m-cal-dow" }, [
                                (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.weekdayNames, (name) => {
                                  return (_openBlock(), _createElementBlock("div", {
                                    key: name,
                                    class: "col text-center text-caption m-muted"
                                  }, _toDisplayString(name), 1))
                                }), 128))
                              ]),
                              (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.monthWeeks, (week) => {
                                return (_openBlock(), _createElementBlock("div", {
                                  key: week.key,
                                  class: "row no-wrap m-cal-week"
                                }, [
                                  (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(week.days, (cell) => {
                                    return (_openBlock(), _createElementBlock("div", {
                                      key: cell.day,
                                      class: _normalizeClass(["col m-cal-cell", {'m-cal-cell--out': !cell.inMonth, 'm-cal-cell--sel': _ctx.selection && _ctx.selection.day === cell.day}]),
                                      "data-day": cell.day,
                                      "data-zone": "month",
                                      onPointerdown: $event => (_ctx.onCellDown($event, cell.day, 'month', null))
                                    }, [
                                      _createElementVNode("div", { class: "m-cal-daynum" }, [
                                        _createElementVNode("span", {
                                          class: _normalizeClass({'m-cal-today': cell.isToday}),
                                          onPointerdown: _withModifiers(() => {}, ["stop"]),
                                          onClick: _withModifiers($event => (_ctx.goDay(cell.day)), ["stop"])
                                        }, _toDisplayString(cell.dayNum), 43, ["onPointerdown", "onClick"])
                                      ]),
                                      (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(cell.items.slice(0, cell.limit), (ev) => {
                                        return (_openBlock(), _createElementBlock("div", {
                                          key: ev.key,
                                          class: _normalizeClass(["m-cal-chip ellipsis", _ctx.chipClass(ev)]),
                                          "data-index": ev.index,
                                          onPointerdown: _withModifiers($event => (_ctx.onEventDown($event, ev, 'move')), ["stop"])
                                        }, [
                                          (ev.allDay)
                                            ? (_openBlock(), _createElementBlock(_Fragment, { key: 0 }, [
                                                _createTextVNode(_toDisplayString(ev.title), 1)
                                              ], 64))
                                            : (_openBlock(), _createElementBlock(_Fragment, { key: 1 }, [
                                                _createElementVNode("span", {
                                                  class: _normalizeClass(["m-cal-dot", 'bg-' + ev.color])
                                                }, null, 2),
                                                _createElementVNode("span", { class: "m-cal-chip-time" }, _toDisplayString(ev.shortTime), 1),
                                                _createTextVNode(" " + _toDisplayString(ev.title), 1)
                                              ], 64))
                                        ], 42, ["data-index", "onPointerdown"]))
                                      }), 128)),
                                      (cell.items.length > cell.limit)
                                        ? (_openBlock(), _createElementBlock("div", {
                                            key: 0,
                                            class: "m-cal-more text-primary",
                                            onPointerdown: _withModifiers(() => {}, ["stop"]),
                                            onClick: _withModifiers($event => (_ctx.goDay(cell.day)), ["stop"])
                                          }, "+" + _toDisplayString(cell.items.length - cell.limit) + " " + _toDisplayString(_ctx.labels.more), 41, ["onPointerdown", "onClick"]))
                                        : _createCommentVNode("", true)
                                    ], 42, ["data-day", "onPointerdown"]))
                                  }), 128))
                                ]))
                              }), 128))
                            ]),
                            _: 1
                          }))
                        : (_ctx.calMode === 'week' || _ctx.calMode === 'day')
                          ? (_openBlock(), _createBlock(_component_q_card, {
                              key: 1,
                              flat: "",
                              bordered: "",
                              class: "m-cal-grid"
                            }, {
                              default: _withCtx(() => [
                                _createElementVNode("div", { class: "row no-wrap m-cal-head" }, [
                                  _createElementVNode("div", { class: "m-cal-gutter" }),
                                  (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.gridCols, (col) => {
                                    return (_openBlock(), _createElementBlock("div", {
                                      key: col.key,
                                      class: _normalizeClass(["col m-cal-head-cell text-center ellipsis", {'m-cal-head-cell--today': col.isToday}]),
                                      onClick: $event => (col.resource === null ? _ctx.goDay(col.day) : null)
                                    }, [
                                      _createElementVNode("div", {
                                        class: _normalizeClass(["text-caption", col.isToday ? 'text-primary text-weight-medium' : 'm-muted'])
                                      }, _toDisplayString(col.top), 3),
                                      _createElementVNode("div", { class: "m-cal-head-label" }, _toDisplayString(col.label), 1)
                                    ], 10, ["onClick"]))
                                  }), 128))
                                ]),
                                _createElementVNode("div", { class: "row no-wrap m-cal-allday" }, [
                                  _createElementVNode("div", { class: "m-cal-gutter text-caption m-muted" }, _toDisplayString(_ctx.labels.allDay), 1),
                                  (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.gridCols, (col) => {
                                    return (_openBlock(), _createElementBlock("div", {
                                      key: col.key,
                                      class: "col m-cal-allday-cell",
                                      "data-day": col.day,
                                      "data-res": col.resource,
                                      "data-zone": "allday",
                                      onPointerdown: $event => (_ctx.onCellDown($event, col.day, 'allday', col.resource))
                                    }, [
                                      (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(col.allDay, (ev) => {
                                        return (_openBlock(), _createElementBlock("div", {
                                          key: ev.key,
                                          class: _normalizeClass(["m-cal-chip ellipsis", _ctx.chipClass(ev)]),
                                          "data-index": ev.index,
                                          onPointerdown: _withModifiers($event => (_ctx.onEventDown($event, ev, 'move')), ["stop"])
                                        }, _toDisplayString(ev.title), 43, ["data-index", "onPointerdown"]))
                                      }), 128))
                                    ], 40, ["data-day", "data-res", "onPointerdown"]))
                                  }), 128))
                                ]),
                                _createElementVNode("div", {
                                  ref: "calScroll",
                                  class: "m-cal-scroll"
                                }, [
                                  _createElementVNode("div", {
                                    class: "row no-wrap m-cal-hours",
                                    style: _normalizeStyle({height: (24 * _ctx.hourPx) + 'px'})
                                  }, [
                                    _createElementVNode("div", { class: "m-cal-gutter relative-position" }, [
                                      (_openBlock(), _createElementBlock(_Fragment, null, _renderList(23, (hour) => {
                                        return _createElementVNode("div", {
                                          key: hour,
                                          class: "m-cal-hour-label text-caption m-muted",
                                          style: _normalizeStyle({top: (hour * _ctx.hourPx - 8) + 'px'})
                                        }, _toDisplayString(_ctx.hourLabel(hour)), 5)
                                      }), 64))
                                    ]),
                                    (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.gridCols, (col) => {
                                      return (_openBlock(), _createElementBlock("div", {
                                        key: col.key,
                                        class: "col m-cal-col",
                                        style: _normalizeStyle({backgroundSize: '100% ' + _ctx.hourPx + 'px'}),
                                        "data-day": col.day,
                                        "data-res": col.resource,
                                        "data-zone": "time",
                                        onPointerdown: $event => (_ctx.onCellDown($event, col.day, 'time', col.resource))
                                      }, [
                                        (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(col.timed, (place) => {
                                          return (_openBlock(), _createElementBlock("div", {
                                            key: place.ev.key,
                                            class: _normalizeClass(["m-cal-event", _ctx.eventClass(place.ev)]),
                                            style: _normalizeStyle(_ctx.eventStyle(place)),
                                            "data-index": place.ev.index,
                                            onPointerdown: _withModifiers($event => (_ctx.onEventDown($event, place.ev, 'move')), ["stop"])
                                          }, [
                                            _createElementVNode("div", { class: "m-cal-event-title ellipsis" }, _toDisplayString(place.ev.title), 1),
                                            ((place.ev.end - place.ev.start) >= 45)
                                              ? (_openBlock(), _createElementBlock("div", {
                                                  key: 0,
                                                  class: "m-cal-event-time ellipsis"
                                                }, _toDisplayString(place.ev.timeText), 1))
                                              : _createCommentVNode("", true),
                                            (_ctx.canMove && !_ctx.phone)
                                              ? (_openBlock(), _createElementBlock("div", {
                                                  key: 1,
                                                  class: "m-cal-resize",
                                                  onPointerdown: _withModifiers($event => (_ctx.onEventDown($event, place.ev, 'resize')), ["stop"])
                                                }, null, 40, ["onPointerdown"]))
                                              : _createCommentVNode("", true)
                                          ], 46, ["data-index", "onPointerdown"]))
                                        }), 128)),
                                        (_ctx.selection && _ctx.selection.zone === 'time' && _ctx.selection.col === col.key)
                                          ? (_openBlock(), _createElementBlock("div", {
                                              key: 0,
                                              class: "m-cal-selection",
                                              style: _normalizeStyle(_ctx.spanStyle(_ctx.selection.start, _ctx.selection.end, col.day))
                                            }, _toDisplayString(_ctx.selectionText), 5))
                                          : _createCommentVNode("", true),
                                        (col.isToday)
                                          ? (_openBlock(), _createElementBlock("div", {
                                              key: 1,
                                              class: "m-cal-now",
                                              style: _normalizeStyle({top: (_ctx.nowMinutes / 60 * _ctx.hourPx) + 'px'})
                                            }, null, 4))
                                          : _createCommentVNode("", true)
                                      ], 44, ["data-day", "data-res", "onPointerdown"]))
                                    }), 128))
                                  ], 4)
                                ], 512)
                              ]),
                              _: 1
                            }))
                          : (_openBlock(), _createBlock(_component_q_card, {
                              key: 2,
                              flat: "",
                              bordered: "",
                              class: "m-cal-agenda"
                            }, {
                              default: _withCtx(() => [
                                (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.agenda, (group, groupIndex) => {
                                  return (_openBlock(), _createElementBlock(_Fragment, {
                                    key: group.day
                                  }, [
                                    (groupIndex > 0)
                                      ? (_openBlock(), _createBlock(_component_q_separator, { key: 0 }))
                                      : _createCommentVNode("", true),
                                    _createElementVNode("div", {
                                      class: _normalizeClass(["m-cal-agenda-day text-subtitle2 q-px-md q-pt-sm", group.isToday ? 'text-primary' : ''])
                                    }, _toDisplayString(group.label), 3),
                                    _createVNode(_component_q_list, {
                                      dense: "",
                                      class: "q-pb-xs"
                                    }, {
                                      default: _withCtx(() => [
                                        (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(group.items, (ev) => {
                                          return (_openBlock(), _createBlock(_component_q_item, {
                                            key: ev.key,
                                            clickable: "",
                                            class: "m-cal-agenda-item",
                                            "data-index": ev.index,
                                            onClick: $event => (_ctx.openEvent($event, ev))
                                          }, {
                                            default: _withCtx(() => [
                                              _createVNode(_component_q_item_section, {
                                                side: "",
                                                class: "m-cal-agenda-time text-caption"
                                              }, {
                                                default: _withCtx(() => [
                                                  _createTextVNode(_toDisplayString(ev.allDay ? _ctx.labels.allDay : (_ctx.phone ? ev.shortTime : ev.timeText)), 1)
                                                ]),
                                                _: 2
                                              }, 1024),
                                              _createVNode(_component_q_item_section, {
                                                avatar: "",
                                                class: "m-cal-agenda-dot"
                                              }, {
                                                default: _withCtx(() => [
                                                  _createElementVNode("span", {
                                                    class: _normalizeClass(["m-cal-dot", 'bg-' + ev.color])
                                                  }, null, 2)
                                                ]),
                                                _: 2
                                              }, 1024),
                                              _createVNode(_component_q_item_section, null, {
                                                default: _withCtx(() => [
                                                  _createVNode(_component_q_item_label, null, {
                                                    default: _withCtx(() => [
                                                      _createTextVNode(_toDisplayString(ev.title), 1)
                                                    ]),
                                                    _: 2
                                                  }, 1024),
                                                  (ev.resourceLabel)
                                                    ? (_openBlock(), _createBlock(_component_q_item_label, {
                                                        key: 0,
                                                        caption: ""
                                                      }, {
                                                        default: _withCtx(() => [
                                                          _createTextVNode(_toDisplayString(ev.resourceLabel), 1)
                                                        ]),
                                                        _: 2
                                                      }, 1024))
                                                    : _createCommentVNode("", true)
                                                ]),
                                                _: 2
                                              }, 1024)
                                            ]),
                                            _: 2
                                          }, 1032, ["data-index", "onClick"]))
                                        }), 128))
                                      ]),
                                      _: 2
                                    }, 1024)
                                  ], 64))
                                }), 128)),
                                (!_ctx.agenda.length)
                                  ? (_openBlock(), _createElementBlock("div", {
                                      key: 0,
                                      class: "q-pa-md m-muted"
                                    }, _toDisplayString(_ctx.labels.noEvents), 1))
                                  : _createCommentVNode("", true)
                              ]),
                              _: 1
                            })),
                      _createElementVNode("div", {
                        id: _ctx.uid + '_anchor',
                        class: "m-cal-anchor",
                        style: _normalizeStyle({left: _ctx.anchorRect.left + 'px', top: _ctx.anchorRect.top + 'px', width: _ctx.anchorRect.width + 'px', height: _ctx.anchorRect.height + 'px'})
                      }, null, 12, ["id"]),
                      _createVNode(_component_q_menu, {
                        modelValue: _ctx.pop.open,
                        "onUpdate:modelValue": $event => ((_ctx.pop.open) = $event),
                        target: '#' + _ctx.uid + '_anchor',
                        "no-parent-event": "",
                        anchor: "bottom left",
                        self: "top left",
                        class: "m-cal-pop"
                      }, {
                        default: _withCtx(() => [
                          (_ctx.pop.index !== null)
                            ? (_openBlock(), _createElementBlock("div", {
                                key: 0,
                                class: "m-cal-pop-body q-pa-sm"
                              }, [
                                _createElementVNode("div", { class: "text-caption m-muted q-mb-xs" }, _toDisplayString(_ctx.pop.when), 1),
                                _renderSlot(_ctx.$slots, 'card_' + _ctx.pop.index, {
                                  index: _ctx.pop.index,
                                  row: _ctx.popRow
                                }, () => [
                                  _renderSlot(_ctx.$slots, "card", {
                                    index: _ctx.pop.index,
                                    row: _ctx.popRow
                                  }, () => [
                                    _createElementVNode("div", { class: "m-card-title" }, _toDisplayString(_ctx.cardTitle(_ctx.popRow)), 1)
                                  ])
                                ])
                              ]))
                            : _createCommentVNode("", true)
                        ]),
                        _: 3
                      }, 8, ["modelValue", "onUpdate:modelValue", "target"]),
                      _createVNode(_component_m_dialog, {
                        modelValue: _ctx.create.open,
                        "onUpdate:modelValue": $event => ((_ctx.create.open) = $event),
                        title: _ctx.labels.newEvent,
                        width: "520"
                      }, {
                        default: _withCtx(() => [
                          (_ctx.create.open)
                            ? (_openBlock(), _createElementBlock(_Fragment, { key: 0 }, [
                                (_ctx.create.url)
                                  ? (_openBlock(), _createBlock(_component_m_dynamic_container, {
                                      key: 0,
                                      id: _ctx.uid + '_create',
                                      url: _ctx.create.url
                                    }, null, 8, ["id", "url"]))
                                  : (_openBlock(), _createBlock(_component_m_form, {
                                      key: 1,
                                      action: _ctx.calendar.createTransition,
                                      "fields-initial": _ctx.create.initial,
                                      onSubmitted: _ctx.afterCreate
                                    }, {
                                      default: _withCtx((formProps) => [
                                        (_ctx.calendar.titleField)
                                          ? (_openBlock(), _createBlock(_component_m_text_line, {
                                              key: 0,
                                              modelValue: formProps.fields[_ctx.calendar.titleField],
                                              "onUpdate:modelValue": $event => ((formProps.fields[_ctx.calendar.titleField]) = $event),
                                              name: _ctx.calendar.titleField,
                                              id: _ctx.uid + '_ctitle',
                                              label: _ctx.labels.title,
                                              required: ""
                                            }, null, 8, ["modelValue", "onUpdate:modelValue", "name", "id", "label"]))
                                          : _createCommentVNode("", true),
                                        _createElementVNode("div", { class: "row q-col-gutter-sm q-mt-xs" }, [
                                          _createElementVNode("div", { class: "col-12 col-sm-6" }, [
                                            _createVNode(_component_m_date_time, {
                                              modelValue: formProps.fields[_ctx.startName],
                                              "onUpdate:modelValue": $event => ((formProps.fields[_ctx.startName]) = $event),
                                              name: _ctx.startName,
                                              id: _ctx.uid + '_cstart',
                                              type: _ctx.create.dateType,
                                              label: _ctx.labels.starts
                                            }, null, 8, ["modelValue", "onUpdate:modelValue", "name", "id", "type", "label"])
                                          ]),
                                          (_ctx.calendar.endField)
                                            ? (_openBlock(), _createElementBlock("div", {
                                                key: 0,
                                                class: "col-12 col-sm-6"
                                              }, [
                                                _createVNode(_component_m_date_time, {
                                                  modelValue: formProps.fields[_ctx.endName],
                                                  "onUpdate:modelValue": $event => ((formProps.fields[_ctx.endName]) = $event),
                                                  name: _ctx.endName,
                                                  id: _ctx.uid + '_cend',
                                                  type: _ctx.create.dateType,
                                                  label: _ctx.labels.ends
                                                }, null, 8, ["modelValue", "onUpdate:modelValue", "name", "id", "type", "label"])
                                              ]))
                                            : _createCommentVNode("", true)
                                        ]),
                                        _createElementVNode("div", { class: "row justify-end q-mt-md" }, [
                                          _createVNode(_component_q_btn, {
                                            unelevated: "",
                                            dense: "",
                                            "no-caps": "",
                                            color: "primary",
                                            type: "submit",
                                            class: "q-px-md m-cal-create-submit",
                                            label: _ctx.labels.create
                                          }, null, 8, ["label"])
                                        ])
                                      ]),
                                      _: 1
                                    }, 8, ["action", "fields-initial", "onSubmitted"]))
                              ], 64))
                            : _createCommentVNode("", true)
                        ]),
                        _: 1
                      }, 8, ["modelValue", "onUpdate:modelValue", "title"])
                    ], 2))
                  : _createCommentVNode("", true)
              ], 10, ["id", "data-view"]))
            }
        } },
        "m-markdown": { hash: "ca4e1146", make: function (Vue) {
            const { toDisplayString: _toDisplayString, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createElementBlock("div", {
                class: "m-markdown",
                id: _ctx.id,
                onClick: _ctx.onClick
              }, [
                (_ctx.failed)
                  ? (_openBlock(), _createElementBlock("div", {
                      key: 0,
                      class: "m-markdown-plain"
                    }, _toDisplayString(_ctx.text), 1))
                  : (_openBlock(), _createElementBlock("div", {
                      key: 1,
                      class: "m-markdown-body",
                      innerHTML: _ctx.html
                    }, null, 8, ["innerHTML"]))
              ], 8, ["id", "onClick"]))
            }
        } },
        "m-menu-icon": { hash: "1d47e7c6", make: function (Vue) {
            const { normalizeClass: _normalizeClass, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, resolveComponent: _resolveComponent, createBlock: _createBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_icon = _resolveComponent("q-icon")

              return (_ctx.isClass)
                ? (_openBlock(), _createElementBlock("i", {
                    key: 0,
                    class: _normalizeClass(_ctx.name)
                  }, null, 2))
                : (_openBlock(), _createBlock(_component_q_icon, {
                    key: 1,
                    name: _ctx.name,
                    size: _ctx.size
                  }, null, 8, ["name", "size"]))
            }
        } },
        "m-menu-item-content": { hash: "29c35951", make: function (Vue) {
            const { resolveComponent: _resolveComponent, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createElementBlock: _createElementBlock, withCtx: _withCtx, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, normalizeClass: _normalizeClass, createVNode: _createVNode, Fragment: _Fragment } = Vue

            return function render(_ctx, _cache) {
              const _component_m_menu_icon = _resolveComponent("m-menu-icon")
              const _component_q_item_section = _resolveComponent("q-item-section")

              return (_openBlock(), _createElementBlock(_Fragment, null, [
                (_ctx.menuItem.image)
                  ? (_openBlock(), _createBlock(_component_q_item_section, {
                      key: 0,
                      avatar: ""
                    }, {
                      default: _withCtx(() => [
                        (_ctx.menuItem.imageType === 'icon')
                          ? (_openBlock(), _createBlock(_component_m_menu_icon, {
                              key: 0,
                              name: _ctx.menuItem.image,
                              size: "xs"
                            }, null, 8, ["name"]))
                          : (_openBlock(), _createElementBlock("img", {
                              key: 1,
                              src: _ctx.menuItem.image,
                              alt: _ctx.menuItem.title,
                              height: "14",
                              class: "invertible"
                            }, null, 8, ["src", "alt"]))
                      ]),
                      _: 1
                    }))
                  : _createCommentVNode("", true),
                _createVNode(_component_q_item_section, {
                  class: _normalizeClass({'text-primary text-weight-medium':_ctx.active})
                }, {
                  default: _withCtx(() => [
                    _createTextVNode(_toDisplayString(_ctx.menuItem.title), 1)
                  ]),
                  _: 1
                }, 8, ["class"])
              ], 64))
            }
        } },
        "m-menu-nav-item": { hash: "a691a42d", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, renderList: _renderList, Fragment: _Fragment, openBlock: _openBlock, createElementBlock: _createElementBlock, createBlock: _createBlock, withCtx: _withCtx, createCommentVNode: _createCommentVNode } = Vue

            return function render(_ctx, _cache) {
              const _component_m_menu_item_content = _resolveComponent("m-menu-item-content")
              const _component_m_menu_subscreen_item = _resolveComponent("m-menu-subscreen-item")
              const _component_q_expansion_item = _resolveComponent("q-expansion-item")
              const _component_m_menu_nav_item = _resolveComponent("m-menu-nav-item")

              return (_ctx.navMenuItem && _ctx.navMenuItem.subscreens && _ctx.navMenuItem.subscreens.length)
                ? (_openBlock(), _createBlock(_component_q_expansion_item, {
                    key: 0,
                    "model-value": true,
                    "content-inset-level": 0.3,
                    "switch-toggle-side": "",
                    dense: "",
                    "dense-toggle": "",
                    "expand-icon-class": "text-grey-6",
                    "expanded-icon": "arrow_drop_down",
                    to: _ctx.navMenuItem.pathWithParams
                  }, {
                    header: _withCtx(() => [
                      _createVNode(_component_m_menu_item_content, {
                        "menu-item": _ctx.navMenuItem,
                        active: ""
                      }, null, 8, ["menu-item"])
                    ]),
                    default: _withCtx(() => [
                      (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.navMenuItem.subscreens, (subscreen, ssIndex) => {
                        return (_openBlock(), _createBlock(_component_m_menu_subscreen_item, {
                          key: subscreen.name,
                          "menu-index": _ctx.menuIndex,
                          "subscreen-index": ssIndex
                        }, null, 8, ["menu-index", "subscreen-index"]))
                      }), 128))
                    ]),
                    _: 1
                  }, 8, ["to"]))
                : (_ctx.navMenuItem && _ctx.navMenuItem.savedFinds && _ctx.navMenuItem.savedFinds.length)
                  ? (_openBlock(), _createBlock(_component_q_expansion_item, {
                      key: 1,
                      "model-value": true,
                      "content-inset-level": 0.3,
                      "switch-toggle-side": "",
                      dense: "",
                      "dense-toggle": "",
                      "expand-icon-class": "text-grey-6",
                      "expanded-icon": "arrow_drop_down",
                      to: _ctx.navMenuItem.pathWithParams
                    }, {
                      header: _withCtx(() => [
                        _createVNode(_component_m_menu_item_content, {
                          "menu-item": _ctx.navMenuItem,
                          active: ""
                        }, null, 8, ["menu-item"])
                      ]),
                      default: _withCtx(() => [
                        (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.navMenuItem.savedFinds, (savedFind, ssIndex) => {
                          return (_openBlock(), _createBlock(_component_q_expansion_item, {
                            key: savedFind.name,
                            "model-value": false,
                            "switch-toggle-side": "",
                            dense: "",
                            "dense-toggle": "",
                            "expand-icon-class": "text-grey-6",
                            "expand-icon": "chevron_right",
                            to: savedFind.pathWithParams
                          }, {
                            header: _withCtx(() => [
                              _createVNode(_component_m_menu_item_content, {
                                "menu-item": savedFind,
                                active: savedFind.active
                              }, null, 8, ["menu-item", "active"])
                            ]),
                            _: 2
                          }, 1032, ["to"]))
                        }), 128))
                      ]),
                      _: 1
                    }, 8, ["to"]))
                  : (_ctx.navMenuItem && _ctx.menuIndex < (_ctx.navMenuLength - 1))
                    ? (_openBlock(), _createBlock(_component_q_expansion_item, {
                        key: 2,
                        "model-value": true,
                        "content-inset-level": 0.3,
                        "switch-toggle-side": "",
                        dense: "",
                        "dense-toggle": "",
                        "expand-icon-class": "text-grey-6",
                        "expanded-icon": "arrow_drop_down",
                        to: _ctx.navMenuItem.pathWithParams
                      }, {
                        header: _withCtx(() => [
                          _createVNode(_component_m_menu_item_content, {
                            "menu-item": _ctx.navMenuItem,
                            active: ""
                          }, null, 8, ["menu-item"])
                        ]),
                        default: _withCtx(() => [
                          _createVNode(_component_m_menu_nav_item, {
                            "menu-index": _ctx.menuIndex + 1
                          }, null, 8, ["menu-index"])
                        ]),
                        _: 1
                      }, 8, ["to"]))
                    : (_ctx.navMenuItem)
                      ? (_openBlock(), _createBlock(_component_q_expansion_item, {
                          key: 3,
                          "model-value": false,
                          "switch-toggle-side": "",
                          dense: "",
                          "dense-toggle": "",
                          "expand-icon-class": "text-grey-6",
                          "expand-icon": "arrow_right",
                          to: _ctx.navMenuItem.pathWithParams
                        }, {
                          header: _withCtx(() => [
                            _createVNode(_component_m_menu_item_content, {
                              "menu-item": _ctx.navMenuItem,
                              active: ""
                            }, null, 8, ["menu-item"])
                          ]),
                          _: 1
                        }, 8, ["to"]))
                      : _createCommentVNode("", true)
            }
        } },
        "m-menu-subscreen-item": { hash: "caf03d9", make: function (Vue) {
            const { resolveComponent: _resolveComponent, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createVNode: _createVNode, withCtx: _withCtx } = Vue

            return function render(_ctx, _cache) {
              const _component_m_menu_nav_item = _resolveComponent("m-menu-nav-item")
              const _component_m_menu_item_content = _resolveComponent("m-menu-item-content")
              const _component_q_expansion_item = _resolveComponent("q-expansion-item")

              return (_ctx.subscreen.active)
                ? (_openBlock(), _createBlock(_component_m_menu_nav_item, {
                    key: 0,
                    "menu-index": _ctx.menuIndex + 1
                  }, null, 8, ["menu-index"]))
                : (_openBlock(), _createBlock(_component_q_expansion_item, {
                    key: 1,
                    "model-value": false,
                    "switch-toggle-side": "",
                    dense: "",
                    "dense-toggle": "",
                    "expand-icon-class": "text-grey-6",
                    "expand-icon": "arrow_right",
                    to: _ctx.subscreen.pathWithParams
                  }, {
                    header: _withCtx(() => [
                      _createVNode(_component_m_menu_item_content, { "menu-item": _ctx.subscreen }, null, 8, ["menu-item"])
                    ]),
                    _: 1
                  }, 8, ["to"]))
            }
        } },
        "m-mermaid": { hash: "981f6b40", make: function (Vue) {
            const { renderSlot: _renderSlot, createElementVNode: _createElementVNode, normalizeStyle: _normalizeStyle, openBlock: _openBlock, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createElementBlock("div", {
                class: "mermaid-container",
                style: _normalizeStyle({height:_ctx.height,width:_ctx.width})
              }, [
                _createElementVNode("pre", {
                  ref: "source",
                  style: {"display":"none"}
                }, [
                  _renderSlot(_ctx.$slots, "default")
                ], 512),
                _createElementVNode("div", { ref: "target" }, null, 512)
              ], 4))
            }
        } },
        "m-number-card": { hash: "6622601f", make: function (Vue) {
            const { toDisplayString: _toDisplayString, createElementVNode: _createElementVNode, normalizeClass: _normalizeClass, resolveComponent: _resolveComponent, createVNode: _createVNode, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, createBlock: _createBlock, withCtx: _withCtx, resolveDynamicComponent: _resolveDynamicComponent } = Vue

            return function render(_ctx, _cache) {
              const _component_q_icon = _resolveComponent("q-icon")
              const _component_m_menu_icon = _resolveComponent("m-menu-icon")
              const _component_q_card_section = _resolveComponent("q-card-section")
              const _component_q_card = _resolveComponent("q-card")

              return (_openBlock(), _createBlock(_resolveDynamicComponent(_ctx.wrapper), {
                href: _ctx.href || undefined,
                class: _normalizeClass(["m-number-card-wrap", {'m-number-card-link': !!_ctx.href}])
              }, {
                default: _withCtx(() => [
                  _createVNode(_component_q_card, {
                    flat: "",
                    bordered: "",
                    class: _normalizeClass(["m-number-card", 'm-number-card-' + _ctx.size])
                  }, {
                    default: _withCtx(() => [
                      _createVNode(_component_q_card_section, { class: "row no-wrap items-start q-pa-md" }, {
                        default: _withCtx(() => [
                          _createElementVNode("div", { class: "col m-number-card-body" }, [
                            _createElementVNode("div", { class: "text-overline m-number-card-title ellipsis" }, _toDisplayString(_ctx.title), 1),
                            _createElementVNode("div", {
                              class: _normalizeClass(["m-number-card-value", _ctx.valueClass])
                            }, _toDisplayString(_ctx.shownValue), 3),
                            (_ctx.deltaInfo)
                              ? (_openBlock(), _createElementBlock("div", {
                                  key: 0,
                                  class: _normalizeClass(["text-caption row no-wrap items-center m-number-card-delta", _ctx.deltaClass])
                                }, [
                                  _createVNode(_component_q_icon, {
                                    name: _ctx.deltaInfo.icon,
                                    size: "xs",
                                    class: "q-mr-xs"
                                  }, null, 8, ["name"]),
                                  _createElementVNode("span", null, _toDisplayString(_ctx.delta), 1)
                                ], 2))
                              : _createCommentVNode("", true)
                          ]),
                          (_ctx.icon)
                            ? (_openBlock(), _createBlock(_component_m_menu_icon, {
                                key: 0,
                                name: _ctx.icon,
                                size: "sm",
                                class: "m-number-card-icon"
                              }, null, 8, ["name"]))
                            : _createCommentVNode("", true)
                        ]),
                        _: 1
                      })
                    ]),
                    _: 1
                  }, 8, ["class"])
                ]),
                _: 1
              }, 8, ["href", "class"]))
            }
        } },
        "m-qapps-root": { hash: "f304ab9b", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, createBlock: _createBlock, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, withCtx: _withCtx, renderList: _renderList, Fragment: _Fragment, createElementVNode: _createElementVNode, normalizeClass: _normalizeClass, resolveDirective: _resolveDirective, withDirectives: _withDirectives, resolveDynamicComponent: _resolveDynamicComponent, withModifiers: _withModifiers } = Vue

            return function render(_ctx, _cache) {
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_icon = _resolveComponent("q-icon")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_m_menu_icon = _resolveComponent("m-menu-icon")
              const _component_q_separator = _resolveComponent("q-separator")
              const _component_q_menu = _resolveComponent("q-menu")
              const _component_q_toolbar_title = _resolveComponent("q-toolbar-title")
              const _component_q_breadcrumbs_el = _resolveComponent("q-breadcrumbs-el")
              const _component_q_item_section = _resolveComponent("q-item-section")
              const _component_q_item = _resolveComponent("q-item")
              const _component_q_list = _resolveComponent("q-list")
              const _component_q_breadcrumbs = _resolveComponent("q-breadcrumbs")
              const _component_q_space = _resolveComponent("q-space")
              const _component_q_badge = _resolveComponent("q-badge")
              const _component_m_dynamic_dialog = _resolveComponent("m-dynamic-dialog")
              const _component_q_item_label = _resolveComponent("q-item-label")
              const _component_m_avatar = _resolveComponent("m-avatar")
              const _component_m_user_status_menu = _resolveComponent("m-user-status-menu")
              const _component_q_toggle = _resolveComponent("q-toggle")
              const _component_q_toolbar = _resolveComponent("q-toolbar")
              const _component_q_header = _resolveComponent("q-header")
              const _component_m_menu_nav_item = _resolveComponent("m-menu-nav-item")
              const _component_q_scroll_area = _resolveComponent("q-scroll-area")
              const _component_q_drawer = _resolveComponent("q-drawer")
              const _component_m_subscreens_active = _resolveComponent("m-subscreens-active")
              const _component_q_page = _resolveComponent("q-page")
              const _component_m_shell_tray = _resolveComponent("m-shell-tray")
              const _component_q_page_container = _resolveComponent("q-page-container")
              const _component_q_footer = _resolveComponent("q-footer")
              const _component_q_layout = _resolveComponent("q-layout")
              const _component_m_text_line = _resolveComponent("m-text-line")
              const _component_q_form = _resolveComponent("q-form")
              const _component_m_dialog = _resolveComponent("m-dialog")
              const _component_m_shell_palette = _resolveComponent("m-shell-palette")
              const _component_m_shell_quick_create = _resolveComponent("m-shell-quick-create")
              const _component_m_shell_keys = _resolveComponent("m-shell-keys")
              const _component_m_user_status_dialog = _resolveComponent("m-user-status-dialog")
              const _directive_close_popup = _resolveDirective("close-popup")

              return (_openBlock(), _createElementBlock(_Fragment, null, [
                _createVNode(_component_q_layout, { view: "hHh LpR fFf" }, {
                  default: _withCtx(() => [
                    _createVNode(_component_q_header, {
                      bordered: "",
                      id: "top",
                      class: _normalizeClass(_ctx.$q.dark.isActive ? 'bg-dark text-white' : 'bg-white text-dark')
                    }, {
                      default: _withCtx(() => [
                        _createVNode(_component_q_toolbar, { dense: "" }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_btn, {
                              flat: "",
                              round: "",
                              dense: "",
                              icon: "menu",
                              onClick: $event => (_ctx.toggleLeftOpen())
                            }, null, 8, ["onClick"]),
                            _createVNode(_component_q_btn, {
                              flat: "",
                              dense: "",
                              "no-caps": "",
                              id: "m-launcher",
                              class: "q-mx-xs",
                              "aria-label": _ctx.labels.applications
                            }, {
                              default: _withCtx(() => [
                                (_ctx.shell.logoUrl)
                                  ? (_openBlock(), _createElementBlock("img", {
                                      key: 0,
                                      src: _ctx.shell.logoUrl,
                                      alt: "",
                                      height: "28"
                                    }, null, 8, ["src"]))
                                  : (_openBlock(), _createBlock(_component_q_icon, {
                                      key: 1,
                                      name: "sym_o_apps"
                                    })),
                                _createVNode(_component_q_tooltip, null, {
                                  default: _withCtx(() => [
                                    _createTextVNode(_toDisplayString(_ctx.labels.applications), 1)
                                  ]),
                                  _: 1
                                }),
                                _createVNode(_component_q_menu, {
                                  anchor: "bottom left",
                                  self: "top left",
                                  class: "m-launcher"
                                }, {
                                  default: _withCtx(() => [
                                    _createElementVNode("div", { class: "q-pa-sm" }, [
                                      _createElementVNode("div", { class: "m-launcher-grid" }, [
                                        (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.shellApps, (app) => {
                                          return _withDirectives((_openBlock(), _createBlock(_component_q_btn, {
                                            key: app.name,
                                            unelevated: "",
                                            "no-caps": "",
                                            class: _normalizeClass(["m-btn-tonal m-launcher-app", {'m-launcher-current': _ctx.currentApp && app.name === _ctx.currentApp.name}]),
                                            onClick: $event => (_ctx.setUrl(app.path))
                                          }, {
                                            default: _withCtx(() => [
                                              _createElementVNode("div", { class: "column items-center no-wrap full-width" }, [
                                                (app.image && app.imageType === 'icon')
                                                  ? (_openBlock(), _createBlock(_component_m_menu_icon, {
                                                      key: 0,
                                                      name: app.image,
                                                      size: "md"
                                                    }, null, 8, ["name"]))
                                                  : (app.image)
                                                    ? (_openBlock(), _createElementBlock("img", {
                                                        key: 1,
                                                        src: app.image,
                                                        alt: app.title,
                                                        height: "28",
                                                        class: "invertible"
                                                      }, null, 8, ["src", "alt"]))
                                                    : (_openBlock(), _createBlock(_component_q_icon, {
                                                        key: 2,
                                                        name: "sym_o_apps",
                                                        size: "md"
                                                      })),
                                                _createElementVNode("div", { class: "m-launcher-title" }, _toDisplayString(app.title), 1)
                                              ])
                                            ]),
                                            _: 2
                                          }, 1032, ["class", "onClick"])), [
                                            [_directive_close_popup]
                                          ])
                                        }), 128))
                                      ]),
                                      _createVNode(_component_q_separator, { class: "q-my-sm" }),
                                      _withDirectives(_createVNode(_component_q_btn, {
                                        flat: "",
                                        "no-caps": "",
                                        dense: "",
                                        icon: "sym_o_apps",
                                        label: _ctx.labels.allApplications,
                                        onClick: $event => (_ctx.setUrl(_ctx.linkBasePath + '/AppList'))
                                      }, null, 8, ["label", "onClick"]), [
                                        [_directive_close_popup]
                                      ])
                                    ])
                                  ]),
                                  _: 1
                                })
                              ]),
                              _: 1
                            }, 8, ["aria-label"]),
                            (_ctx.headerTitle)
                              ? (_openBlock(), _createBlock(_component_q_toolbar_title, {
                                  key: 0,
                                  shrink: "",
                                  class: "gt-xs text-subtitle1 text-weight-medium q-pr-sm"
                                }, {
                                  default: _withCtx(() => [
                                    _createTextVNode(_toDisplayString(_ctx.headerTitle), 1)
                                  ]),
                                  _: 1
                                }))
                              : _createCommentVNode("", true),
                            _createVNode(_component_q_breadcrumbs, {
                              id: "m-breadcrumbs",
                              class: "gt-xs",
                              gutter: "xs",
                              "separator-color": "grey-5",
                              "active-color": "grey-8"
                            }, {
                              default: _withCtx(() => [
                                (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.navMenuList, (navMenuItem, menuIndex) => {
                                  return (_openBlock(), _createElementBlock(_Fragment, {
                                    key: navMenuItem.path || menuIndex
                                  }, [
                                    (menuIndex >= _ctx.breadcrumbStart)
                                      ? (_openBlock(), _createElementBlock(_Fragment, { key: 0 }, [
                                          (menuIndex === _ctx.navMenuList.length - 1)
                                            ? (_openBlock(), _createBlock(_component_q_breadcrumbs_el, {
                                                key: 0,
                                                label: navMenuItem.title,
                                                class: "text-weight-medium"
                                              }, null, 8, ["label"]))
                                            : (navMenuItem.subscreens && navMenuItem.subscreens.length && !navMenuItem.hasTabMenu)
                                              ? (_openBlock(), _createBlock(_component_q_breadcrumbs_el, {
                                                  key: 1,
                                                  class: "cursor-pointer"
                                                }, {
                                                  default: _withCtx(() => [
                                                    _createTextVNode(_toDisplayString(navMenuItem.title), 1),
                                                    _createVNode(_component_q_icon, {
                                                      name: "arrow_drop_down",
                                                      size: "xs"
                                                    }),
                                                    _createVNode(_component_q_menu, {
                                                      anchor: "bottom left",
                                                      self: "top left"
                                                    }, {
                                                      default: _withCtx(() => [
                                                        _createVNode(_component_q_list, {
                                                          dense: "",
                                                          style: {"min-width":"200px"}
                                                        }, {
                                                          default: _withCtx(() => [
                                                            (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(navMenuItem.subscreens, (subscreen) => {
                                                              return _withDirectives((_openBlock(), _createBlock(_component_q_item, {
                                                                key: subscreen.name,
                                                                clickable: "",
                                                                active: subscreen.active,
                                                                "active-class": "text-primary text-weight-medium",
                                                                to: subscreen.pathWithParams
                                                              }, {
                                                                default: _withCtx(() => [
                                                                  _createVNode(_component_q_item_section, { avatar: "" }, {
                                                                    default: _withCtx(() => [
                                                                      (subscreen.image && subscreen.imageType === 'icon')
                                                                        ? (_openBlock(), _createBlock(_component_m_menu_icon, {
                                                                            key: 0,
                                                                            name: subscreen.image,
                                                                            size: "xs"
                                                                          }, null, 8, ["name"]))
                                                                        : (subscreen.image)
                                                                          ? (_openBlock(), _createElementBlock("img", {
                                                                              key: 1,
                                                                              src: subscreen.image,
                                                                              alt: subscreen.title,
                                                                              width: "18",
                                                                              class: "invertible"
                                                                            }, null, 8, ["src", "alt"]))
                                                                          : (_openBlock(), _createBlock(_component_q_icon, {
                                                                              key: 2,
                                                                              name: "chevron_right",
                                                                              size: "xs"
                                                                            }))
                                                                    ]),
                                                                    _: 2
                                                                  }, 1024),
                                                                  _createVNode(_component_q_item_section, null, {
                                                                    default: _withCtx(() => [
                                                                      _createTextVNode(_toDisplayString(subscreen.title), 1)
                                                                    ]),
                                                                    _: 2
                                                                  }, 1024)
                                                                ]),
                                                                _: 2
                                                              }, 1032, ["active", "to"])), [
                                                                [_directive_close_popup]
                                                              ])
                                                            }), 128))
                                                          ]),
                                                          _: 2
                                                        }, 1024)
                                                      ]),
                                                      _: 2
                                                    }, 1024)
                                                  ]),
                                                  _: 2
                                                }, 1024))
                                              : (_openBlock(), _createBlock(_component_q_breadcrumbs_el, {
                                                  key: 2,
                                                  label: navMenuItem.title,
                                                  to: _ctx.getNavHref(menuIndex)
                                                }, null, 8, ["label", "to"]))
                                        ], 64))
                                      : _createCommentVNode("", true)
                                  ], 64))
                                }), 128))
                              ]),
                              _: 1
                            }),
                            _createVNode(_component_q_space),
                            _createVNode(_component_q_btn, {
                              flat: "",
                              dense: "",
                              "no-caps": "",
                              id: "m-search",
                              icon: "sym_o_search",
                              round: !_ctx.$q.screen.gt.sm,
                              label: _ctx.$q.screen.gt.sm ? _ctx.labels.search : undefined,
                              onClick: $event => (_ctx.openPalette())
                            }, {
                              default: _withCtx(() => [
                                (_ctx.$q.screen.gt.sm)
                                  ? (_openBlock(), _createElementBlock("span", {
                                      key: 0,
                                      class: "m-kbd q-ml-sm"
                                    }, _toDisplayString(_ctx.paletteKeyHint), 1))
                                  : _createCommentVNode("", true),
                                _createVNode(_component_q_tooltip, null, {
                                  default: _withCtx(() => [
                                    _createTextVNode(_toDisplayString(_ctx.labels.search) + " (" + _toDisplayString(_ctx.paletteKeyHint) + ")", 1)
                                  ]),
                                  _: 1
                                })
                              ]),
                              _: 1
                            }, 8, ["round", "label", "onClick"]),
                            (_ctx.quickCreateItems.length)
                              ? (_openBlock(), _createBlock(_component_q_btn, {
                                  key: 1,
                                  flat: "",
                                  round: "",
                                  dense: "",
                                  id: "m-quick-create-menu",
                                  icon: "sym_o_add"
                                }, {
                                  default: _withCtx(() => [
                                    _createVNode(_component_q_tooltip, null, {
                                      default: _withCtx(() => [
                                        _createTextVNode(_toDisplayString(_ctx.labels.quickCreate), 1)
                                      ]),
                                      _: 1
                                    }),
                                    _createVNode(_component_q_menu, {
                                      anchor: "bottom right",
                                      self: "top right"
                                    }, {
                                      default: _withCtx(() => [
                                        _createVNode(_component_q_list, {
                                          dense: "",
                                          style: {"min-width":"220px"}
                                        }, {
                                          default: _withCtx(() => [
                                            (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.quickCreateItems, (item) => {
                                              return _withDirectives((_openBlock(), _createBlock(_component_q_item, {
                                                key: item.key,
                                                clickable: "",
                                                onClick: $event => (_ctx.openQuickCreate(item))
                                              }, {
                                                default: _withCtx(() => [
                                                  _createVNode(_component_q_item_section, { avatar: "" }, {
                                                    default: _withCtx(() => [
                                                      _createVNode(_component_m_menu_icon, {
                                                        name: item.icon || 'sym_o_add',
                                                        size: "xs"
                                                      }, null, 8, ["name"])
                                                    ]),
                                                    _: 2
                                                  }, 1024),
                                                  _createVNode(_component_q_item_section, null, {
                                                    default: _withCtx(() => [
                                                      _createTextVNode(_toDisplayString(item.label), 1)
                                                    ]),
                                                    _: 2
                                                  }, 1024)
                                                ]),
                                                _: 2
                                              }, 1032, ["onClick"])), [
                                                [_directive_close_popup]
                                              ])
                                            }), 128))
                                          ]),
                                          _: 1
                                        })
                                      ]),
                                      _: 1
                                    })
                                  ]),
                                  _: 1
                                }))
                              : _createCommentVNode("", true),
                            (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.headerCounters, (counter) => {
                              return (_openBlock(), _createBlock(_component_q_btn, {
                                key: counter.key,
                                flat: "",
                                round: "",
                                dense: "",
                                id: 'm-counter-' + counter.key,
                                icon: counter.icon,
                                class: _normalizeClass({'text-primary': counter.tray && _ctx.trayOpen && (counter.any || _ctx.trayTab === counter.key)}),
                                onClick: $event => (counter.tray ? _ctx.toggleTray(counter.any ? null : counter.key) : _ctx.setUrl(counter.url))
                              }, {
                                default: _withCtx(() => [
                                  (_ctx.counterCount(counter))
                                    ? (_openBlock(), _createBlock(_component_q_badge, {
                                        key: 0,
                                        floating: "",
                                        rounded: "",
                                        color: "negative"
                                      }, {
                                        default: _withCtx(() => [
                                          _createTextVNode(_toDisplayString(_ctx.counterText(counter)), 1)
                                        ]),
                                        _: 2
                                      }, 1024))
                                    : _createCommentVNode("", true),
                                  (counter.key === 'chat' && _ctx.myPresence)
                                    ? (_openBlock(), _createElementBlock("span", {
                                        key: 1,
                                        class: _normalizeClass(["m-avatar-presence m-shell-presence", 'm-presence-' + _ctx.myPresence])
                                      }, null, 2))
                                    : _createCommentVNode("", true),
                                  _createVNode(_component_q_tooltip, null, {
                                    default: _withCtx(() => [
                                      _createTextVNode(_toDisplayString(counter.label), 1)
                                    ]),
                                    _: 2
                                  }, 1024)
                                ]),
                                _: 2
                              }, 1032, ["id", "icon", "class", "onClick"]))
                            }), 128)),
                            (_openBlock(), _createBlock(_resolveDynamicComponent(_ctx.qzVue), { ref: "qzVue" }, null, 512)),
                            (_ctx.documentMenuList.length)
                              ? (_openBlock(), _createBlock(_component_q_btn, {
                                  key: 2,
                                  flat: "",
                                  round: "",
                                  dense: "",
                                  icon: "help_outline"
                                }, {
                                  default: _withCtx(() => [
                                    _createVNode(_component_q_tooltip, null, {
                                      default: _withCtx(() => [
                                        _createTextVNode(_toDisplayString(_ctx.labels.documentation), 1)
                                      ]),
                                      _: 1
                                    }),
                                    _createVNode(_component_q_menu, null, {
                                      default: _withCtx(() => [
                                        _createVNode(_component_q_list, {
                                          dense: "",
                                          style: {"min-width":"200px"}
                                        }, {
                                          default: _withCtx(() => [
                                            (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.documentMenuList, (screenDoc) => {
                                              return (_openBlock(), _createBlock(_component_q_item, {
                                                key: screenDoc.index
                                              }, {
                                                default: _withCtx(() => [
                                                  _createVNode(_component_q_item_section, null, {
                                                    default: _withCtx(() => [
                                                      _createVNode(_component_m_dynamic_dialog, {
                                                        url: _ctx.currentPath + '/screenDoc?docIndex=' + screenDoc.index,
                                                        "button-text": screenDoc.title,
                                                        title: screenDoc.title
                                                      }, null, 8, ["url", "button-text", "title"])
                                                    ]),
                                                    _: 2
                                                  }, 1024)
                                                ]),
                                                _: 2
                                              }, 1024))
                                            }), 128))
                                          ]),
                                          _: 1
                                        })
                                      ]),
                                      _: 1
                                    })
                                  ]),
                                  _: 1
                                }))
                              : _createCommentVNode("", true),
                            (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.navPlugins, (navPlugin) => {
                              return (_openBlock(), _createBlock(_resolveDynamicComponent(navPlugin)))
                            }), 256)),
                            (!_ctx.shellValues.trayNotifications)
                              ? (_openBlock(), _createBlock(_component_q_btn, {
                                  key: 3,
                                  flat: "",
                                  round: "",
                                  dense: "",
                                  icon: "notifications"
                                }, {
                                  default: _withCtx(() => [
                                    _createVNode(_component_q_tooltip, null, {
                                      default: _withCtx(() => [
                                        _createTextVNode(_toDisplayString(_ctx.labels.notifyHistory), 1)
                                      ]),
                                      _: 1
                                    }),
                                    _createVNode(_component_q_menu, null, {
                                      default: _withCtx(() => [
                                        _createVNode(_component_q_list, {
                                          dense: "",
                                          separator: "",
                                          style: {"min-width":"320px","max-width":"440px"}
                                        }, {
                                          default: _withCtx(() => [
                                            (!_ctx.notifyHistoryList.length)
                                              ? (_openBlock(), _createBlock(_component_q_item, { key: 0 }, {
                                                  default: _withCtx(() => [
                                                    _createVNode(_component_q_item_section, { class: "text-grey-6" }, {
                                                      default: _withCtx(() => [
                                                        _createTextVNode(_toDisplayString(_ctx.labels.notifyHistory), 1)
                                                      ]),
                                                      _: 1
                                                    })
                                                  ]),
                                                  _: 1
                                                }))
                                              : _createCommentVNode("", true),
                                            (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.notifyHistoryList, (histItem, histIndex) => {
                                              return (_openBlock(), _createBlock(_component_q_item, {
                                                key: histIndex,
                                                clickable: !!histItem.link,
                                                onClick: $event => (histItem.link && _ctx.moqui.openLink(histItem.link))
                                              }, {
                                                default: _withCtx(() => [
                                                  _createVNode(_component_q_item_section, { avatar: "" }, {
                                                    default: _withCtx(() => [
                                                      _createVNode(_component_q_icon, {
                                                        name: _ctx.notifyIcon(histItem.type),
                                                        color: _ctx.getQuasarColor(histItem.type) || 'info'
                                                      }, null, 8, ["name", "color"])
                                                    ]),
                                                    _: 2
                                                  }, 1024),
                                                  _createVNode(_component_q_item_section, null, {
                                                    default: _withCtx(() => [
                                                      _createVNode(_component_q_item_label, null, {
                                                        default: _withCtx(() => [
                                                          _createTextVNode(_toDisplayString(histItem.message), 1)
                                                        ]),
                                                        _: 2
                                                      }, 1024),
                                                      _createVNode(_component_q_item_label, { caption: "" }, {
                                                        default: _withCtx(() => [
                                                          _createTextVNode(_toDisplayString(histItem.time), 1)
                                                        ]),
                                                        _: 2
                                                      }, 1024)
                                                    ]),
                                                    _: 2
                                                  }, 1024)
                                                ]),
                                                _: 2
                                              }, 1032, ["clickable", "onClick"]))
                                            }), 128))
                                          ]),
                                          _: 1
                                        })
                                      ]),
                                      _: 1
                                    })
                                  ]),
                                  _: 1
                                }))
                              : _createCommentVNode("", true),
                            _createVNode(_component_q_btn, {
                              flat: "",
                              round: "",
                              dense: "",
                              id: "m-account"
                            }, {
                              default: _withCtx(() => [
                                _createVNode(_component_m_avatar, {
                                  name: _ctx.userFullName || _ctx.username,
                                  "user-id": _ctx.userId,
                                  size: "28px"
                                }, null, 8, ["name", "user-id"]),
                                _createVNode(_component_q_tooltip, null, {
                                  default: _withCtx(() => [
                                    _createTextVNode(_toDisplayString(_ctx.userFullName || _ctx.labels.account), 1)
                                  ]),
                                  _: 1
                                }),
                                _createVNode(_component_q_menu, null, {
                                  default: _withCtx(() => [
                                    _createVNode(_component_q_list, { style: {"min-width":"240px"} }, {
                                      default: _withCtx(() => [
                                        (_ctx.userFullName || _ctx.username)
                                          ? (_openBlock(), _createBlock(_component_q_item, { key: 0 }, {
                                              default: _withCtx(() => [
                                                _createVNode(_component_q_item_section, null, {
                                                  default: _withCtx(() => [
                                                    _createVNode(_component_q_item_label, { class: "text-weight-medium" }, {
                                                      default: _withCtx(() => [
                                                        _createTextVNode(_toDisplayString(_ctx.userFullName || _ctx.username), 1)
                                                      ]),
                                                      _: 1
                                                    }),
                                                    (_ctx.userFullName)
                                                      ? (_openBlock(), _createBlock(_component_q_item_label, {
                                                          key: 0,
                                                          caption: ""
                                                        }, {
                                                          default: _withCtx(() => [
                                                            _createTextVNode(_toDisplayString(_ctx.username), 1)
                                                          ]),
                                                          _: 1
                                                        }))
                                                      : _createCommentVNode("", true)
                                                  ]),
                                                  _: 1
                                                })
                                              ]),
                                              _: 1
                                            }))
                                          : _createCommentVNode("", true),
                                        (_ctx.userFullName || _ctx.username)
                                          ? (_openBlock(), _createBlock(_component_q_separator, { key: 1 }))
                                          : _createCommentVNode("", true),
                                        (_ctx.chatStatus)
                                          ? (_openBlock(), _createBlock(_component_m_user_status_menu, { key: 2 }))
                                          : _createCommentVNode("", true),
                                        (_ctx.chatStatus)
                                          ? (_openBlock(), _createBlock(_component_q_separator, { key: 3 }))
                                          : _createCommentVNode("", true),
                                        (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.accountMenuItems, (item) => {
                                          return _withDirectives((_openBlock(), _createBlock(_component_q_item, {
                                            key: item.url,
                                            clickable: "",
                                            onClick: $event => (_ctx.setUrl(item.url))
                                          }, {
                                            default: _withCtx(() => [
                                              _createVNode(_component_q_item_section, { avatar: "" }, {
                                                default: _withCtx(() => [
                                                  _createVNode(_component_m_menu_icon, {
                                                    name: item.icon || 'sym_o_chevron_right'
                                                  }, null, 8, ["name"])
                                                ]),
                                                _: 2
                                              }, 1024),
                                              _createVNode(_component_q_item_section, null, {
                                                default: _withCtx(() => [
                                                  _createTextVNode(_toDisplayString(item.label), 1)
                                                ]),
                                                _: 2
                                              }, 1024)
                                            ]),
                                            _: 2
                                          }, 1032, ["onClick"])), [
                                            [_directive_close_popup]
                                          ])
                                        }), 128)),
                                        (_ctx.accountMenuItems.length)
                                          ? (_openBlock(), _createBlock(_component_q_separator, { key: 4 }))
                                          : _createCommentVNode("", true),
                                        (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.accountPlugins, (accountPlugin, accountIndex) => {
                                          return (_openBlock(), _createBlock(_component_q_item, { key: accountIndex }, {
                                            default: _withCtx(() => [
                                              _createVNode(_component_q_item_section, null, {
                                                default: _withCtx(() => [
                                                  (_openBlock(), _createBlock(_resolveDynamicComponent(accountPlugin)))
                                                ]),
                                                _: 2
                                              }, 1024)
                                            ]),
                                            _: 2
                                          }, 1024))
                                        }), 128)),
                                        _createVNode(_component_q_item, {
                                          tag: "label",
                                          clickable: ""
                                        }, {
                                          default: _withCtx(() => [
                                            _createVNode(_component_q_item_section, { avatar: "" }, {
                                              default: _withCtx(() => [
                                                _createVNode(_component_q_icon, { name: "dark_mode" })
                                              ]),
                                              _: 1
                                            }),
                                            _createVNode(_component_q_item_section, null, {
                                              default: _withCtx(() => [
                                                _createTextVNode(_toDisplayString(_ctx.labels.switchDarkLight), 1)
                                              ]),
                                              _: 1
                                            }),
                                            _createVNode(_component_q_item_section, { side: "" }, {
                                              default: _withCtx(() => [
                                                _createVNode(_component_q_toggle, {
                                                  dense: "",
                                                  "model-value": _ctx.$q.dark.isActive,
                                                  "onUpdate:modelValue": $event => (_ctx.switchDarkLight())
                                                }, null, 8, ["model-value", "onUpdate:modelValue"])
                                              ]),
                                              _: 1
                                            })
                                          ]),
                                          _: 1
                                        }),
                                        _withDirectives((_openBlock(), _createBlock(_component_q_item, {
                                          clickable: "",
                                          onClick: _ctx.reLoginShowDialog
                                        }, {
                                          default: _withCtx(() => [
                                            _createVNode(_component_q_item_section, { avatar: "" }, {
                                              default: _withCtx(() => [
                                                _createVNode(_component_q_icon, { name: "autorenew" })
                                              ]),
                                              _: 1
                                            }),
                                            _createVNode(_component_q_item_section, null, {
                                              default: _withCtx(() => [
                                                _createTextVNode(_toDisplayString(_ctx.labels.reLogin), 1)
                                              ]),
                                              _: 1
                                            })
                                          ]),
                                          _: 1
                                        }, 8, ["onClick"])), [
                                          [_directive_close_popup]
                                        ]),
                                        _createVNode(_component_q_separator),
                                        _createVNode(_component_q_item, {
                                          clickable: "",
                                          tag: "a",
                                          href: _ctx.shell.logoutUrl,
                                          "data-confirm": _ctx.labels.logout + ' ' + _ctx.userFullName + '?'
                                        }, {
                                          default: _withCtx(() => [
                                            _createVNode(_component_q_item_section, { avatar: "" }, {
                                              default: _withCtx(() => [
                                                _createVNode(_component_q_icon, {
                                                  name: "logout",
                                                  color: "negative"
                                                })
                                              ]),
                                              _: 1
                                            }),
                                            _createVNode(_component_q_item_section, { class: "text-negative" }, {
                                              default: _withCtx(() => [
                                                _createTextVNode(_toDisplayString(_ctx.labels.logout), 1)
                                              ]),
                                              _: 1
                                            })
                                          ]),
                                          _: 1
                                        }, 8, ["href", "data-confirm"])
                                      ]),
                                      _: 1
                                    })
                                  ]),
                                  _: 1
                                })
                              ]),
                              _: 1
                            })
                          ]),
                          _: 1
                        })
                      ]),
                      _: 1
                    }, 8, ["class"]),
                    _createVNode(_component_q_drawer, {
                      modelValue: _ctx.leftOpen,
                      "onUpdate:modelValue": $event => ((_ctx.leftOpen) = $event),
                      side: "left",
                      bordered: "",
                      width: 260,
                      breakpoint: 1023
                    }, {
                      default: _withCtx(() => [
                        _createVNode(_component_q_scroll_area, { class: "fit" }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_list, {
                              dense: "",
                              padding: ""
                            }, {
                              default: _withCtx(() => [
                                _createVNode(_component_m_menu_nav_item, { "menu-index": 0 })
                              ]),
                              _: 1
                            })
                          ]),
                          _: 1
                        })
                      ]),
                      _: 1
                    }, 8, ["modelValue", "onUpdate:modelValue"]),
                    _createVNode(_component_q_page_container, { id: "m-canvas" }, {
                      default: _withCtx(() => [
                        _createVNode(_component_q_page, { class: "q-pa-sm" }, {
                          default: _withCtx(() => [
                            _createVNode(_component_m_subscreens_active)
                          ]),
                          _: 1
                        }),
                        (_ctx.trayTabs.length)
                          ? (_openBlock(), _createBlock(_component_m_shell_tray, { key: 0 }))
                          : _createCommentVNode("", true)
                      ]),
                      _: 1
                    }),
                    _createVNode(_component_q_footer, {
                      bordered: "",
                      id: "footer",
                      class: _normalizeClass(["text-caption q-pa-xs", _ctx.$q.dark.isActive ? 'bg-dark text-grey-5' : 'bg-grey-2 text-grey-8'])
                    }, {
                      default: _withCtx(() => [
                        _createElementVNode("div", {
                          innerHTML: _ctx.shell.footerHtml,
                          class: "row full-width"
                        }, null, 8, ["innerHTML"])
                      ]),
                      _: 1
                    }, 8, ["class"])
                  ]),
                  _: 1
                }),
                _createVNode(_component_m_dialog, {
                  modelValue: _ctx.reLoginShow,
                  "onUpdate:modelValue": $event => ((_ctx.reLoginShow) = $event),
                  width: "400",
                  title: _ctx.labels.reLogin
                }, {
                  default: _withCtx(() => [
                    (_ctx.reLoginMfaData)
                      ? (_openBlock(), _createElementBlock("div", { key: 0 }, [
                          _createElementVNode("div", { class: "text-center q-pb-sm" }, [
                            _createTextVNode("User "),
                            _createElementVNode("strong", null, _toDisplayString(_ctx.username), 1),
                            _createTextVNode(" requires an authentication code, you have these options:")
                          ]),
                          _createElementVNode("div", { class: "text-center q-pb-sm" }, _toDisplayString(_ctx.reLoginMfaData.factorTypeDescriptions.join(", ")), 1),
                          _createVNode(_component_q_form, {
                            onSubmit: _withModifiers(_ctx.reLoginVerifyOtp, ["prevent"]),
                            autocapitalize: "off",
                            autocomplete: "off",
                            class: "q-gutter-y-sm"
                          }, {
                            default: _withCtx(() => [
                              _createVNode(_component_m_text_line, {
                                modelValue: _ctx.reLoginOtp,
                                "onUpdate:modelValue": $event => ((_ctx.reLoginOtp) = $event),
                                name: "code",
                                type: "password",
                                label: _ctx.labels.authenticationCode
                              }, null, 8, ["modelValue", "onUpdate:modelValue", "label"]),
                              _createVNode(_component_q_btn, {
                                unelevated: "",
                                "no-caps": "",
                                color: "primary",
                                type: "submit",
                                label: _ctx.labels.signIn
                              }, null, 8, ["label"])
                            ]),
                            _: 1
                          }, 8, ["onSubmit"]),
                          (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.reLoginMfaData.sendableFactors, (sendableFactor) => {
                            return (_openBlock(), _createElementBlock("div", {
                              key: sendableFactor.factorId,
                              class: "q-pt-sm"
                            }, [
                              _createVNode(_component_q_btn, {
                                unelevated: "",
                                "no-caps": "",
                                dense: "",
                                class: "m-btn-tonal",
                                label: _ctx.labels.sendCodeTo + ' ' + sendableFactor.factorOption,
                                onClick: _withModifiers($event => (_ctx.reLoginSendOtp(sendableFactor.factorId)), ["prevent"])
                              }, null, 8, ["label", "onClick"])
                            ]))
                          }), 128))
                        ]))
                      : (_openBlock(), _createElementBlock("div", { key: 1 }, [
                          _createElementVNode("div", { class: "text-center q-pb-sm" }, [
                            _createTextVNode("Please sign in to continue as user "),
                            _createElementVNode("strong", null, _toDisplayString(_ctx.username), 1)
                          ]),
                          _createVNode(_component_q_form, {
                            onSubmit: _withModifiers(_ctx.reLoginSubmit, ["prevent"]),
                            autocapitalize: "off",
                            autocomplete: "off",
                            class: "q-gutter-y-sm"
                          }, {
                            default: _withCtx(() => [
                              _createVNode(_component_m_text_line, {
                                modelValue: _ctx.reLoginPassword,
                                "onUpdate:modelValue": $event => ((_ctx.reLoginPassword) = $event),
                                name: "password",
                                type: "password",
                                label: _ctx.labels.password
                              }, null, 8, ["modelValue", "onUpdate:modelValue", "label"]),
                              _createElementVNode("div", null, [
                                _createVNode(_component_q_btn, {
                                  unelevated: "",
                                  "no-caps": "",
                                  color: "primary",
                                  type: "submit",
                                  label: _ctx.labels.signIn
                                }, null, 8, ["label"]),
                                _createVNode(_component_q_btn, {
                                  flat: "",
                                  "no-caps": "",
                                  color: "negative",
                                  onClick: _withModifiers(_ctx.reLoginReload, ["prevent"]),
                                  label: _ctx.labels.reloadPage
                                }, null, 8, ["onClick", "label"])
                              ])
                            ]),
                            _: 1
                          }, 8, ["onSubmit"])
                        ]))
                  ]),
                  _: 1
                }, 8, ["modelValue", "onUpdate:modelValue", "title"]),
                _createVNode(_component_m_shell_palette),
                _createVNode(_component_m_shell_quick_create),
                _createVNode(_component_m_shell_keys),
                (_ctx.chatStatus)
                  ? (_openBlock(), _createBlock(_component_m_user_status_dialog, { key: 0 }))
                  : _createCommentVNode("", true)
              ], 64))
            }
        } },
        "m-radio": { hash: "a9ce2610", make: function (Vue) {
            const { toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createVNode: _createVNode, mergeProps: _mergeProps } = Vue

            return function render(_ctx, _cache) {
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_option_group = _resolveComponent("q-option-group")
              const _component_m_field = _resolveComponent("m-field")

              return (_openBlock(), _createBlock(_component_m_field, _mergeProps({
                borderless: "",
                class: "m-field-group",
                label: _ctx.label,
                "bg-color": _ctx.bg,
                disable: _ctx.disable
              }, _ctx.errorAttrs), {
                default: _withCtx(() => [
                  (_ctx.tooltip)
                    ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                        ]),
                        _: 1
                      }))
                    : _createCommentVNode("", true),
                  _createVNode(_component_q_option_group, {
                    type: "radio",
                    inline: "",
                    dense: "",
                    size: "sm",
                    options: _ctx.options,
                    "model-value": _ctx.local,
                    "onUpdate:modelValue": _ctx.onInput,
                    name: _ctx.name,
                    id: _ctx.id,
                    disable: _ctx.disable
                  }, null, 8, ["options", "model-value", "onUpdate:modelValue", "name", "id", "disable"])
                ]),
                _: 1
              }, 16, ["label", "bg-color", "disable"]))
            }
        } },
        "m-range-find": { hash: "c79ddf98", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, openBlock: _openBlock, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_m_text_line = _resolveComponent("m-text-line")

              return (_openBlock(), _createElementBlock("div", { class: "row" }, [
                _createVNode(_component_m_text_line, {
                  label: _ctx.label + ' From',
                  name: _ctx.name + '_from',
                  id: _ctx.id + '_from',
                  size: _ctx.size,
                  maxlength: _ctx.maxlength,
                  form: _ctx.form,
                  tooltip: _ctx.tooltip,
                  modelValue: _ctx.fields[_ctx.name + '_from'],
                  "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name + '_from']) = $event)
                }, null, 8, ["label", "name", "id", "size", "maxlength", "form", "tooltip", "modelValue", "onUpdate:modelValue"]),
                _createVNode(_component_m_text_line, {
                  class: "q-pl-xs",
                  label: _ctx.label + ' Thru',
                  name: _ctx.name + '_thru',
                  id: _ctx.id + '_thru',
                  size: _ctx.size,
                  maxlength: _ctx.maxlength,
                  form: _ctx.form,
                  tooltip: _ctx.tooltip,
                  modelValue: _ctx.fields[_ctx.name + '_thru'],
                  "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name + '_thru']) = $event)
                }, null, 8, ["label", "name", "id", "size", "maxlength", "form", "tooltip", "modelValue", "onUpdate:modelValue"])
              ]))
            }
        } },
        "m-record-card": { hash: "a6c7966d", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, withCtx: _withCtx, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createElementBlock: _createElementBlock, renderList: _renderList, Fragment: _Fragment, createElementVNode: _createElementVNode, normalizeClass: _normalizeClass } = Vue

            return function render(_ctx, _cache) {
              const _component_m_menu_icon = _resolveComponent("m-menu-icon")
              const _component_q_item_section = _resolveComponent("q-item-section")
              const _component_m_link = _resolveComponent("m-link")
              const _component_q_item_label = _resolveComponent("q-item-label")
              const _component_q_chip = _resolveComponent("q-chip")
              const _component_q_item = _resolveComponent("q-item")
              const _component_q_skeleton = _resolveComponent("q-skeleton")
              const _component_q_card = _resolveComponent("q-card")

              return (_openBlock(), _createBlock(_component_q_card, {
                flat: "",
                bordered: "",
                class: "m-record-card"
              }, {
                default: _withCtx(() => [
                  (_ctx.info)
                    ? (_openBlock(), _createBlock(_component_q_item, {
                        key: 0,
                        dense: _ctx.dense,
                        class: "m-record-card-item"
                      }, {
                        default: _withCtx(() => [
                          _createVNode(_component_q_item_section, {
                            avatar: "",
                            class: "m-record-card-icon"
                          }, {
                            default: _withCtx(() => [
                              _createVNode(_component_m_menu_icon, {
                                name: _ctx.info.icon,
                                size: "sm"
                              }, null, 8, ["name"])
                            ]),
                            _: 1
                          }),
                          _createVNode(_component_q_item_section, { class: "m-record-card-body" }, {
                            default: _withCtx(() => [
                              _createVNode(_component_q_item_label, { class: "ellipsis text-weight-medium" }, {
                                default: _withCtx(() => [
                                  (_ctx.info.url)
                                    ? (_openBlock(), _createBlock(_component_m_link, {
                                        key: 0,
                                        href: _ctx.info.url,
                                        class: "m-link-text"
                                      }, {
                                        default: _withCtx(() => [
                                          _createTextVNode(_toDisplayString(_ctx.info.title), 1)
                                        ]),
                                        _: 1
                                      }, 8, ["href"]))
                                    : (_openBlock(), _createElementBlock("span", { key: 1 }, _toDisplayString(_ctx.info.title), 1))
                                ]),
                                _: 1
                              }),
                              _createVNode(_component_q_item_label, {
                                caption: "",
                                class: "ellipsis"
                              }, {
                                default: _withCtx(() => [
                                  _createTextVNode(_toDisplayString(_ctx.info.typeLabel), 1),
                                  (_ctx.info.typeLabel && _ctx.info.id)
                                    ? (_openBlock(), _createElementBlock("span", { key: 0 }, " · "))
                                    : _createCommentVNode("", true),
                                  _createTextVNode(_toDisplayString(_ctx.info.id), 1)
                                ]),
                                _: 1
                              }),
                              (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.info.fields, (field) => {
                                return (_openBlock(), _createBlock(_component_q_item_label, {
                                  key: field.label,
                                  caption: "",
                                  class: "ellipsis"
                                }, {
                                  default: _withCtx(() => [
                                    _createElementVNode("span", { class: "m-muted" }, _toDisplayString(field.label), 1),
                                    _createTextVNode(" " + _toDisplayString(field.value), 1)
                                  ]),
                                  _: 2
                                }, 1024))
                              }), 128))
                            ]),
                            _: 1
                          }),
                          (_ctx.info.status)
                            ? (_openBlock(), _createBlock(_component_q_item_section, {
                                key: 0,
                                side: "",
                                top: ""
                              }, {
                                default: _withCtx(() => [
                                  _createVNode(_component_q_chip, {
                                    dense: "",
                                    square: "",
                                    class: _normalizeClass('m-tonal text-' + _ctx.info.status.color)
                                  }, {
                                    default: _withCtx(() => [
                                      _createTextVNode(_toDisplayString(_ctx.info.status.label), 1)
                                    ]),
                                    _: 1
                                  }, 8, ["class"])
                                ]),
                                _: 1
                              }))
                            : _createCommentVNode("", true)
                        ]),
                        _: 1
                      }, 8, ["dense"]))
                    : (_ctx.failed || !_ctx.requestKey)
                      ? (_openBlock(), _createBlock(_component_q_item, {
                          key: 1,
                          dense: _ctx.dense,
                          class: "m-record-card-item"
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_item_section, { class: "m-muted" }, {
                              default: _withCtx(() => [
                                _createTextVNode(_toDisplayString(_ctx.recordEntity) + " " + _toDisplayString(_ctx.recordId), 1)
                              ]),
                              _: 1
                            })
                          ]),
                          _: 1
                        }, 8, ["dense"]))
                      : (_openBlock(), _createBlock(_component_q_item, {
                          key: 2,
                          dense: _ctx.dense,
                          class: "m-record-card-item"
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_item_section, { avatar: "" }, {
                              default: _withCtx(() => [
                                _createVNode(_component_q_skeleton, {
                                  type: "QAvatar",
                                  size: "24px"
                                })
                              ]),
                              _: 1
                            }),
                            _createVNode(_component_q_item_section, null, {
                              default: _withCtx(() => [
                                _createVNode(_component_q_skeleton, {
                                  type: "text",
                                  width: "60%"
                                }),
                                _createVNode(_component_q_skeleton, {
                                  type: "text",
                                  width: "40%"
                                })
                              ]),
                              _: 1
                            })
                          ]),
                          _: 1
                        }, 8, ["dense"]))
                ]),
                _: 1
              }))
            }
        } },
        "m-record-files": { hash: "8482661e", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, normalizeClass: _normalizeClass, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode } = Vue

            return function render(_ctx, _cache) {
              const _component_m_dynamic_container = _resolveComponent("m-dynamic-container")

              return (_ctx.loadUrl)
                ? (_openBlock(), _createElementBlock("div", {
                    key: 0,
                    class: _normalizeClass('m-record-panel m-record-panel-' + _ctx.kind)
                  }, [
                    _createVNode(_component_m_dynamic_container, {
                      ref: "container",
                      id: _ctx.containerId,
                      url: _ctx.loadUrl
                    }, null, 8, ["id", "url"])
                  ], 2))
                : _createCommentVNode("", true)
            }
        } },
        "m-rich-text": { hash: "57823d0d", make: function (Vue) {
            const { resolveComponent: _resolveComponent, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, toDisplayString: _toDisplayString, createElementBlock: _createElementBlock, renderList: _renderList, Fragment: _Fragment, createTextVNode: _createTextVNode, withCtx: _withCtx, createVNode: _createVNode, resolveDirective: _resolveDirective, withDirectives: _withDirectives, withModifiers: _withModifiers, normalizeClass: _normalizeClass, withKeys: _withKeys, normalizeStyle: _normalizeStyle, vShow: _vShow, createElementVNode: _createElementVNode, Teleport: _Teleport } = Vue

            return function render(_ctx, _cache) {
              const _component_m_text_area = _resolveComponent("m-text-area")
              const _component_q_separator = _resolveComponent("q-separator")
              const _component_q_item_section = _resolveComponent("q-item-section")
              const _component_q_item = _resolveComponent("q-item")
              const _component_q_list = _resolveComponent("q-list")
              const _component_q_btn_dropdown = _resolveComponent("q-btn-dropdown")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_input = _resolveComponent("q-input")
              const _component_m_avatar = _resolveComponent("m-avatar")
              const _directive_close_popup = _resolveDirective("close-popup")

              return (_openBlock(), _createElementBlock("div", {
                class: _normalizeClass(["m-rich-text", {'m-rich-text-light': _ctx.light, 'm-rich-text-focused': _ctx.focused}]),
                "data-state": _ctx.state
              }, [
                (_ctx.state === 'failed')
                  ? (_openBlock(), _createBlock(_component_m_text_area, {
                      key: 0,
                      "editor-type": "html",
                      "model-value": _ctx.local,
                      "onUpdate:modelValue": _ctx.onInput,
                      name: _ctx.name,
                      id: _ctx.id,
                      label: _ctx.label,
                      form: _ctx.form,
                      disable: _ctx.disable,
                      readonly: _ctx.readonly,
                      tooltip: _ctx.tooltip
                    }, null, 8, ["model-value", "onUpdate:modelValue", "name", "id", "label", "form", "disable", "readonly", "tooltip"]))
                  : (_openBlock(), _createElementBlock(_Fragment, { key: 1 }, [
                      (_ctx.label)
                        ? (_openBlock(), _createElementBlock("div", {
                            key: 0,
                            class: "text-caption m-muted m-rich-text-label"
                          }, _toDisplayString(_ctx.label), 1))
                        : _createCommentVNode("", true),
                      _createElementVNode("div", {
                        class: _normalizeClass(["m-rich-text-box", [_ctx.bg ? 'bg-' + _ctx.bg : '', _ctx.errorAttrs.error ? 'm-rich-text-error' : '']])
                      }, [
                        (!_ctx.readonly)
                          ? (_openBlock(), _createElementBlock("div", {
                              key: 0,
                              class: "m-rich-text-toolbar row items-center"
                            }, [
                              (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.buttons, (button) => {
                                return (_openBlock(), _createElementBlock(_Fragment, {
                                  key: button.key
                                }, [
                                  (button.separator)
                                    ? (_openBlock(), _createBlock(_component_q_separator, {
                                        key: 0,
                                        vertical: "",
                                        class: "q-mx-xs"
                                      }))
                                    : (button.key === 'block')
                                      ? (_openBlock(), _createBlock(_component_q_btn_dropdown, {
                                          key: 1,
                                          flat: "",
                                          dense: "",
                                          "no-caps": "",
                                          size: "sm",
                                          label: _ctx.t(_ctx.blockLabel),
                                          disable: !_ctx.editable || _ctx.isPlain,
                                          class: "m-rich-text-block",
                                          "content-class": "m-rich-text-menu"
                                        }, {
                                          default: _withCtx(() => [
                                            _createVNode(_component_q_list, { dense: "" }, {
                                              default: _withCtx(() => [
                                                (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.blocks, (block) => {
                                                  return _withDirectives((_openBlock(), _createBlock(_component_q_item, {
                                                    key: block.key,
                                                    clickable: "",
                                                    onClick: $event => (_ctx.run(block.key)),
                                                    active: _ctx.blockLabel === block.label
                                                  }, {
                                                    default: _withCtx(() => [
                                                      _createVNode(_component_q_item_section, null, {
                                                        default: _withCtx(() => [
                                                          _createTextVNode(_toDisplayString(_ctx.t(block.label)), 1)
                                                        ]),
                                                        _: 2
                                                      }, 1024)
                                                    ]),
                                                    _: 2
                                                  }, 1032, ["onClick", "active"])), [
                                                    [_directive_close_popup]
                                                  ])
                                                }), 128))
                                              ]),
                                              _: 1
                                            })
                                          ]),
                                          _: 1
                                        }, 8, ["label", "disable"]))
                                      : (_openBlock(), _createBlock(_component_q_btn, {
                                          key: 2,
                                          flat: "",
                                          dense: "",
                                          size: "sm",
                                          icon: button.icon,
                                          "data-command": button.key,
                                          "aria-label": _ctx.t(button.label),
                                          color: (button.key === 'plain' ? _ctx.isPlain : _ctx.active[button.key]) ? 'primary' : undefined,
                                          class: _normalizeClass({'m-rich-text-on': button.key === 'plain' ? _ctx.isPlain : _ctx.active[button.key]}),
                                          disable: button.key === 'plain' ? (_ctx.disable || _ctx.state !== 'ready') : (!_ctx.editable || _ctx.isPlain),
                                          onMousedown: _withModifiers(() => {}, ["prevent"]),
                                          onClick: $event => (_ctx.run(button.key))
                                        }, {
                                          default: _withCtx(() => [
                                            _createVNode(_component_q_tooltip, null, {
                                              default: _withCtx(() => [
                                                _createTextVNode(_toDisplayString(_ctx.t(button.label)), 1)
                                              ]),
                                              _: 2
                                            }, 1024)
                                          ]),
                                          _: 2
                                        }, 1032, ["icon", "data-command", "aria-label", "color", "class", "disable", "onMousedown", "onClick"]))
                                ], 64))
                              }), 128))
                            ]))
                          : _createCommentVNode("", true),
                        (_ctx.asking)
                          ? (_openBlock(), _createElementBlock("div", {
                              key: 1,
                              class: "m-rich-text-ask row items-center no-wrap q-px-sm q-py-xs q-gutter-x-xs"
                            }, [
                              _createVNode(_component_q_input, {
                                ref: "askInput",
                                dense: "",
                                outlined: "",
                                "hide-bottom-space": "",
                                class: "col",
                                modelValue: _ctx.askValue,
                                "onUpdate:modelValue": $event => ((_ctx.askValue) = $event),
                                placeholder: _ctx.asking === 'image' ? 'https://…/picture.png' : 'https://…',
                                "aria-label": _ctx.t(_ctx.asking === 'image' ? 'Image address' : 'Link address'),
                                onKeydown: [
                                  _withKeys(_withModifiers(_ctx.applyAsk, ["prevent"]), ["enter"]),
                                  _withKeys(_withModifiers($event => (_ctx.asking = null), ["prevent","stop"]), ["esc"])
                                ]
                              }, null, 8, ["modelValue", "onUpdate:modelValue", "placeholder", "aria-label", "onKeydown"]),
                              _createVNode(_component_q_btn, {
                                dense: "",
                                unelevated: "",
                                "no-caps": "",
                                class: "m-btn-tonal",
                                label: _ctx.t('Apply'),
                                onClick: _ctx.applyAsk
                              }, null, 8, ["label", "onClick"]),
                              (_ctx.asking === 'link' && _ctx.active.link)
                                ? (_openBlock(), _createBlock(_component_q_btn, {
                                    key: 0,
                                    dense: "",
                                    flat: "",
                                    "no-caps": "",
                                    label: _ctx.t('Remove'),
                                    onClick: _ctx.removeLink
                                  }, null, 8, ["label", "onClick"]))
                                : _createCommentVNode("", true),
                              _createVNode(_component_q_btn, {
                                dense: "",
                                flat: "",
                                round: "",
                                size: "sm",
                                icon: "sym_o_close",
                                "aria-label": _ctx.t('Close'),
                                onClick: $event => (_ctx.asking = null)
                              }, null, 8, ["aria-label", "onClick"])
                            ]))
                          : _createCommentVNode("", true),
                        (_ctx.isPlain)
                          ? (_openBlock(), _createElementBlock("textarea", {
                              key: 2,
                              id: _ctx.id ? _ctx.id + '_plain' : null,
                              class: "m-rich-text-plain",
                              style: _normalizeStyle({minHeight: _ctx.minHeight}),
                              value: _ctx.local,
                              readonly: _ctx.readonly,
                              disabled: _ctx.disable,
                              onInput: $event => (_ctx.onInput($event.target.value)),
                              onKeydown: _ctx.plainKey,
                              onFocus: $event => (_ctx.focused = true),
                              onBlur: $event => (_ctx.focused = false)
                            }, null, 44, ["id", "value", "readonly", "disabled", "onInput", "onKeydown", "onFocus", "onBlur"]))
                          : _createCommentVNode("", true),
                        _withDirectives(_createElementVNode("div", {
                          ref: "content",
                          class: "m-rich-text-content",
                          style: _normalizeStyle({minHeight: _ctx.minHeight}),
                          onClick: _ctx.focusEnd
                        }, null, 12, ["onClick"]), [
                          [_vShow, !_ctx.isPlain]
                        ]),
                        (_ctx.state === 'loading')
                          ? (_openBlock(), _createElementBlock("div", {
                              key: 3,
                              class: "m-rich-text-loading m-muted text-caption q-px-sm"
                            }, _toDisplayString(_ctx.t('Loading the editor')), 1))
                          : _createCommentVNode("", true)
                      ], 2),
                      (_ctx.errorAttrs.error)
                        ? (_openBlock(), _createElementBlock("div", {
                            key: 1,
                            class: "text-negative text-caption q-px-sm"
                          }, _toDisplayString(_ctx.errorAttrs.errorMessage), 1))
                        : _createCommentVNode("", true),
                      _createElementVNode("input", {
                        type: "hidden",
                        name: _ctx.name,
                        value: _ctx.local,
                        form: _ctx.form
                      }, null, 8, ["name", "value", "form"]),
                      (_openBlock(), _createBlock(_Teleport, { to: "body" }, [
                        _withDirectives(_createElementVNode("div", {
                          ref: "mentionMenu",
                          class: "m-rich-text-mentions",
                          id: _ctx.uid + '-mentions',
                          role: "listbox"
                        }, [
                          _createVNode(_component_q_list, { dense: "" }, {
                            default: _withCtx(() => [
                              (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.mention.items, (item, index) => {
                                return (_openBlock(), _createBlock(_component_q_item, {
                                  key: item.id,
                                  clickable: "",
                                  active: index === _ctx.mention.index,
                                  role: "option",
                                  "aria-selected": index === _ctx.mention.index,
                                  onMousedown: _withModifiers(() => {}, ["prevent"]),
                                  onClick: $event => (_ctx.pickMention(index))
                                }, {
                                  default: _withCtx(() => [
                                    _createVNode(_component_q_item_section, {
                                      avatar: "",
                                      class: "m-rich-text-mention-avatar"
                                    }, {
                                      default: _withCtx(() => [
                                        _createVNode(_component_m_avatar, {
                                          name: item.label,
                                          image: item.image,
                                          size: "22px"
                                        }, null, 8, ["name", "image"])
                                      ]),
                                      _: 2
                                    }, 1024),
                                    _createVNode(_component_q_item_section, null, {
                                      default: _withCtx(() => [
                                        _createTextVNode(_toDisplayString(item.label), 1)
                                      ]),
                                      _: 2
                                    }, 1024)
                                  ]),
                                  _: 2
                                }, 1032, ["active", "aria-selected", "onMousedown", "onClick"]))
                              }), 128)),
                              (!_ctx.mention.items.length)
                                ? (_openBlock(), _createBlock(_component_q_item, { key: 0 }, {
                                    default: _withCtx(() => [
                                      _createVNode(_component_q_item_section, { class: "m-muted" }, {
                                        default: _withCtx(() => [
                                          _createTextVNode(_toDisplayString(_ctx.t(_ctx.mention.loading ? 'Searching' : 'No matches')), 1)
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
                        ], 8, ["id"]), [
                          [_vShow, _ctx.mention.open]
                        ])
                      ]))
                    ], 64))
              ], 10, ["data-state"]))
            }
        } },
        "m-screen-context": { hash: "fcdf7cd5", make: function (Vue) {
            const { openBlock: _openBlock, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createElementBlock("span", { style: {"display":"none"} }))
            }
        } },
        "m-script": { hash: "ddf09c45", make: function (Vue) {
            const { renderSlot: _renderSlot, openBlock: _openBlock, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createElementBlock("div", {
                type: _ctx.type,
                style: {"display":"none"}
              }, [
                _renderSlot(_ctx.$slots, "default")
              ], 8, ["type"]))
            }
        } },
        "m-shell-keys": { hash: "7bb1d715", make: function (Vue) {
            const { renderList: _renderList, Fragment: _Fragment, openBlock: _openBlock, createElementBlock: _createElementBlock, toDisplayString: _toDisplayString, createElementVNode: _createElementVNode, createTextVNode: _createTextVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, createVNode: _createVNode, createBlock: _createBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_item_section = _resolveComponent("q-item-section")
              const _component_q_item = _resolveComponent("q-item")
              const _component_q_list = _resolveComponent("q-list")
              const _component_m_dialog = _resolveComponent("m-dialog")

              return (_openBlock(), _createBlock(_component_m_dialog, {
                "model-value": _ctx.$root.keysHelpShow,
                "onUpdate:modelValue": $event => (_ctx.$root.keysHelpShow = $event),
                id: "m-keys-help",
                title: _ctx.$root.labels.keyboardShortcuts,
                width: "520"
              }, {
                default: _withCtx(() => [
                  (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.groups, (group) => {
                    return (_openBlock(), _createElementBlock("div", {
                      key: group.scope,
                      class: "q-mb-md"
                    }, [
                      _createElementVNode("div", { class: "text-subtitle2 q-mb-xs" }, _toDisplayString(group.title), 1),
                      _createVNode(_component_q_list, {
                        dense: "",
                        bordered: "",
                        separator: "",
                        class: "rounded-borders"
                      }, {
                        default: _withCtx(() => [
                          (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(group.entries, (entry) => {
                            return (_openBlock(), _createBlock(_component_q_item, {
                              key: entry.keys
                            }, {
                              default: _withCtx(() => [
                                _createVNode(_component_q_item_section, null, {
                                  default: _withCtx(() => [
                                    _createTextVNode(_toDisplayString(entry.label), 1)
                                  ]),
                                  _: 2
                                }, 1024),
                                _createVNode(_component_q_item_section, { side: "" }, {
                                  default: _withCtx(() => [
                                    _createElementVNode("span", { class: "m-kbd" }, _toDisplayString(entry.keys), 1)
                                  ]),
                                  _: 2
                                }, 1024)
                              ]),
                              _: 2
                            }, 1024))
                          }), 128))
                        ]),
                        _: 2
                      }, 1024)
                    ]))
                  }), 128))
                ]),
                _: 1
              }, 8, ["model-value", "onUpdate:modelValue", "title"]))
            }
        } },
        "m-shell-palette": { hash: "e4f4e679", make: function (Vue) {
            const { resolveComponent: _resolveComponent, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, withCtx: _withCtx, createVNode: _createVNode, createElementVNode: _createElementVNode, renderList: _renderList, Fragment: _Fragment, createElementBlock: _createElementBlock, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, normalizeClass: _normalizeClass } = Vue

            return function render(_ctx, _cache) {
              const _component_q_spinner = _resolveComponent("q-spinner")
              const _component_q_icon = _resolveComponent("q-icon")
              const _component_m_text_line = _resolveComponent("m-text-line")
              const _component_q_separator = _resolveComponent("q-separator")
              const _component_q_item_label = _resolveComponent("q-item-label")
              const _component_m_menu_icon = _resolveComponent("m-menu-icon")
              const _component_q_item_section = _resolveComponent("q-item-section")
              const _component_q_item = _resolveComponent("q-item")
              const _component_q_list = _resolveComponent("q-list")
              const _component_q_card = _resolveComponent("q-card")
              const _component_q_dialog = _resolveComponent("q-dialog")

              return (_openBlock(), _createBlock(_component_q_dialog, {
                "model-value": _ctx.$root.paletteOpen,
                "onUpdate:modelValue": $event => (_ctx.$root.paletteOpen = $event),
                position: "top",
                onShow: _ctx.onShow,
                onHide: _ctx.onHide
              }, {
                default: _withCtx(() => [
                  _createVNode(_component_q_card, {
                    id: "m-palette",
                    flat: "",
                    bordered: "",
                    class: "m-palette column no-wrap"
                  }, {
                    default: _withCtx(() => [
                      _createElementVNode("div", { class: "q-pa-sm" }, [
                        _createVNode(_component_m_text_line, {
                          modelValue: _ctx.text,
                          "onUpdate:modelValue": $event => ((_ctx.text) = $event),
                          id: "m-palette-input",
                          name: "paletteText",
                          placeholder: _ctx.$root.labels.paletteHint,
                          onKeydown: _ctx.onKey
                        }, {
                          append: _withCtx(() => [
                            (_ctx.searching)
                              ? (_openBlock(), _createBlock(_component_q_spinner, {
                                  key: 0,
                                  size: "xs",
                                  color: "grey-6"
                                }))
                              : (_openBlock(), _createBlock(_component_q_icon, {
                                  key: 1,
                                  name: "sym_o_search",
                                  size: "xs"
                                }))
                          ]),
                          _: 1
                        }, 8, ["modelValue", "onUpdate:modelValue", "placeholder", "onKeydown"])
                      ]),
                      _createVNode(_component_q_separator),
                      _createElementVNode("div", {
                        ref: "results",
                        class: "col scroll m-palette-results"
                      }, [
                        _createVNode(_component_q_list, {
                          dense: "",
                          class: "q-pb-sm"
                        }, {
                          default: _withCtx(() => [
                            (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.groups, (group) => {
                              return (_openBlock(), _createElementBlock(_Fragment, {
                                key: group.key
                              }, [
                                _createVNode(_component_q_item_label, {
                                  header: "",
                                  class: "q-pt-sm q-pb-xs text-weight-medium"
                                }, {
                                  default: _withCtx(() => [
                                    _createTextVNode(_toDisplayString(group.label), 1)
                                  ]),
                                  _: 2
                                }, 1024),
                                (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(group.rows, (row) => {
                                  return (_openBlock(), _createBlock(_component_q_item, {
                                    key: group.key + '_' + row.index,
                                    clickable: "",
                                    active: row.index === _ctx.selected,
                                    "active-class": "m-palette-active",
                                    "data-index": row.index,
                                    onClick: $event => (_ctx.open(row, $event)),
                                    onMousemove: $event => (_ctx.selected = row.index)
                                  }, {
                                    default: _withCtx(() => [
                                      _createVNode(_component_q_item_section, {
                                        avatar: "",
                                        class: "m-palette-icon"
                                      }, {
                                        default: _withCtx(() => [
                                          (row.icon || (row.image && row.imageType === 'icon'))
                                            ? (_openBlock(), _createBlock(_component_m_menu_icon, {
                                                key: 0,
                                                name: row.icon || row.image,
                                                size: "xs"
                                              }, null, 8, ["name"]))
                                            : (row.image)
                                              ? (_openBlock(), _createElementBlock("img", {
                                                  key: 1,
                                                  src: row.image,
                                                  alt: "",
                                                  width: "18",
                                                  class: "invertible"
                                                }, null, 8, ["src"]))
                                              : (_openBlock(), _createBlock(_component_q_icon, {
                                                  key: 2,
                                                  name: "sym_o_chevron_right",
                                                  size: "xs"
                                                }))
                                        ]),
                                        _: 2
                                      }, 1024),
                                      _createVNode(_component_q_item_section, null, {
                                        default: _withCtx(() => [
                                          _createVNode(_component_q_item_label, {
                                            lines: "1",
                                            class: _normalizeClass({'text-primary': row.more})
                                          }, {
                                            default: _withCtx(() => [
                                              _createTextVNode(_toDisplayString(row.title), 1)
                                            ]),
                                            _: 2
                                          }, 1032, ["class"]),
                                          (row.subtitle)
                                            ? (_openBlock(), _createBlock(_component_q_item_label, {
                                                key: 0,
                                                caption: "",
                                                lines: "1"
                                              }, {
                                                default: _withCtx(() => [
                                                  _createTextVNode(_toDisplayString(row.subtitle), 1)
                                                ]),
                                                _: 2
                                              }, 1024))
                                            : _createCommentVNode("", true)
                                        ]),
                                        _: 2
                                      }, 1024)
                                    ]),
                                    _: 2
                                  }, 1032, ["active", "data-index", "onClick", "onMousemove"]))
                                }), 128))
                              ], 64))
                            }), 128)),
                            (!_ctx.rows.length && !_ctx.searching)
                              ? (_openBlock(), _createBlock(_component_q_item, { key: 0 }, {
                                  default: _withCtx(() => [
                                    _createVNode(_component_q_item_section, { class: "m-muted" }, {
                                      default: _withCtx(() => [
                                        _createTextVNode(_toDisplayString(_ctx.query.text ? _ctx.$root.labels.noResults : _ctx.$root.labels.paletteEmpty), 1)
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
                      ], 512),
                      _createVNode(_component_q_separator, { class: "gt-xs" }),
                      _createElementVNode("div", { class: "row items-center q-px-sm q-py-xs text-caption m-muted gt-xs" }, _toDisplayString(_ctx.hint), 1)
                    ]),
                    _: 1
                  })
                ]),
                _: 1
              }, 8, ["model-value", "onUpdate:modelValue", "onShow", "onHide"]))
            }
        } },
        "m-shell-quick-create": { hash: "9b22e439", make: function (Vue) {
            const { resolveComponent: _resolveComponent, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, withCtx: _withCtx } = Vue

            return function render(_ctx, _cache) {
              const _component_m_dynamic_container = _resolveComponent("m-dynamic-container")
              const _component_m_dialog = _resolveComponent("m-dialog")

              return (_openBlock(), _createBlock(_component_m_dialog, {
                "model-value": _ctx.$root.quickCreateShow,
                "onUpdate:modelValue": $event => (_ctx.$root.quickCreateShow = $event),
                id: "m-quick-create",
                title: _ctx.$root.quickCreateTitle,
                width: "560",
                onOnShow: _ctx.onShow
              }, {
                default: _withCtx(() => [
                  (_ctx.$root.quickCreateShow && _ctx.$root.quickCreateUrl)
                    ? (_openBlock(), _createBlock(_component_m_dynamic_container, {
                        key: 0,
                        id: "m-quick-create-body",
                        url: _ctx.$root.quickCreateUrl
                      }, null, 8, ["url"]))
                    : _createCommentVNode("", true)
                ]),
                _: 1
              }, 8, ["model-value", "onUpdate:modelValue", "title", "onOnShow"]))
            }
        } },
        "m-shell-tray": { hash: "613fab4f", make: function (Vue) {
            const { renderList: _renderList, Fragment: _Fragment, openBlock: _openBlock, createElementBlock: _createElementBlock, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createVNode: _createVNode, createElementVNode: _createElementVNode, vShow: _vShow, withDirectives: _withDirectives } = Vue

            return function render(_ctx, _cache) {
              const _component_q_badge = _resolveComponent("q-badge")
              const _component_q_tab = _resolveComponent("q-tab")
              const _component_q_tabs = _resolveComponent("q-tabs")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_separator = _resolveComponent("q-separator")
              const _component_m_dynamic_container = _resolveComponent("m-dynamic-container")
              const _component_q_card = _resolveComponent("q-card")
              const _component_q_page_sticky = _resolveComponent("q-page-sticky")

              return (_ctx.$root.trayMounted)
                ? _withDirectives((_openBlock(), _createBlock(_component_q_page_sticky, {
                    key: 0,
                    position: "right",
                    expand: "",
                    class: "m-tray-sticky"
                  }, {
                    default: _withCtx(() => [
                      _createVNode(_component_q_card, {
                        id: "m-tray",
                        flat: "",
                        bordered: "",
                        square: "",
                        class: "m-tray column no-wrap"
                      }, {
                        default: _withCtx(() => [
                          _createElementVNode("div", { class: "row items-center no-wrap q-pl-xs" }, [
                            _createVNode(_component_q_tabs, {
                              "model-value": _ctx.$root.trayTab,
                              "onUpdate:modelValue": $event => (_ctx.$root.openTray($event)),
                              dense: "",
                              "no-caps": "",
                              "inline-label": "",
                              "narrow-indicator": "",
                              align: "left",
                              class: "col text-grey-7",
                              "active-color": "primary",
                              "indicator-color": "primary"
                            }, {
                              default: _withCtx(() => [
                                (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.$root.trayTabs, (tab) => {
                                  return (_openBlock(), _createBlock(_component_q_tab, {
                                    key: tab.key,
                                    name: tab.key,
                                    icon: tab.icon,
                                    label: tab.label
                                  }, {
                                    default: _withCtx(() => [
                                      (_ctx.$root.countOf(tab.key))
                                        ? (_openBlock(), _createBlock(_component_q_badge, {
                                            key: 0,
                                            color: "negative",
                                            rounded: "",
                                            floating: ""
                                          }, {
                                            default: _withCtx(() => [
                                              _createTextVNode(_toDisplayString(_ctx.$root.countText(tab.key)), 1)
                                            ]),
                                            _: 2
                                          }, 1024))
                                        : _createCommentVNode("", true)
                                    ]),
                                    _: 2
                                  }, 1032, ["name", "icon", "label"]))
                                }), 128))
                              ]),
                              _: 1
                            }, 8, ["model-value", "onUpdate:modelValue"]),
                            _createVNode(_component_q_btn, {
                              flat: "",
                              round: "",
                              dense: "",
                              icon: "sym_o_close",
                              onClick: $event => (_ctx.$root.closeTray())
                            }, {
                              default: _withCtx(() => [
                                _createVNode(_component_q_tooltip, null, {
                                  default: _withCtx(() => [
                                    _createTextVNode(_toDisplayString(_ctx.$root.labels.close), 1)
                                  ]),
                                  _: 1
                                })
                              ]),
                              _: 1
                            }, 8, ["onClick"])
                          ]),
                          _createVNode(_component_q_separator),
                          (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.$root.trayTabs, (tab) => {
                            return _withDirectives((_openBlock(), _createElementBlock("div", {
                              key: tab.key,
                              ref_for: true,
                              ref: (el) => { _ctx.bodies[tab.key] = el; },
                              class: "col scroll q-pa-sm m-tray-body",
                              onScroll: $event => (_ctx.onScroll(tab.key, $event))
                            }, [
                              (_ctx.$root.trayLoaded[tab.key])
                                ? (_openBlock(), _createBlock(_component_m_dynamic_container, {
                                    key: 0,
                                    ref_for: true,
                                    ref: (vm) => { _ctx.containers[tab.key] = vm; },
                                    id: 'm-tray-' + tab.key,
                                    url: tab.loadUrl
                                  }, null, 8, ["id", "url"]))
                                : _createCommentVNode("", true)
                            ], 40, ["onScroll"])), [
                              [_vShow, _ctx.$root.trayTab === tab.key]
                            ])
                          }), 128))
                        ]),
                        _: 1
                      })
                    ]),
                    _: 1
                  }, 512)), [
                    [_vShow, _ctx.$root.trayOpen]
                  ])
                : _createCommentVNode("", true)
            }
        } },
        "m-status-bar": { hash: "bbb0c182", make: function (Vue) {
            const { renderList: _renderList, Fragment: _Fragment, openBlock: _openBlock, createElementBlock: _createElementBlock, resolveComponent: _resolveComponent, createBlock: _createBlock, createCommentVNode: _createCommentVNode, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, mergeProps: _mergeProps, withCtx: _withCtx, createVNode: _createVNode, createElementVNode: _createElementVNode } = Vue

            return function render(_ctx, _cache) {
              const _component_q_icon = _resolveComponent("q-icon")
              const _component_q_chip = _resolveComponent("q-chip")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_m_form = _resolveComponent("m-form")

              return (_openBlock(), _createElementBlock("div", {
                class: "m-status-bar row items-center",
                id: _ctx.id,
                "data-status": _ctx.view.current ? _ctx.view.current.statusId : null
              }, [
                _createElementVNode("div", { class: "m-status-stages row items-center" }, [
                  (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.view.stages, (stage, index) => {
                    return (_openBlock(), _createElementBlock(_Fragment, {
                      key: stage.statusId + ':' + index
                    }, [
                      index
                        ? (_openBlock(), _createBlock(_component_q_icon, {
                            key: 0,
                            name: "sym_o_chevron_right",
                            size: "xs",
                            class: "m-muted m-status-sep"
                          }))
                        : _createCommentVNode("", true),
                      _createVNode(_component_q_chip, _mergeProps({
                        dense: "",
                        square: "",
                        ripple: false
                      }, { ref_for: true }, stage.chip, {
                        "data-stage": stage.statusId,
                        "data-state": stage.state
                      }), {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(stage.label), 1)
                        ]),
                        _: 2
                      }, 1040, ["data-stage", "data-state"])
                    ], 64))
                  }), 128))
                ]),
                (_ctx.action && _ctx.view.buttons.length)
                  ? (_openBlock(), _createElementBlock("div", {
                      key: 0,
                      class: "m-status-actions row items-center"
                    }, [
                      (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.view.buttons, (item) => {
                        return (_openBlock(), _createBlock(_component_m_form, {
                          key: item.toStatusId,
                          action: _ctx.action,
                          "fields-initial": _ctx.fieldsOf(item),
                          class: "m-status-form"
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_btn, _mergeProps({
                              dense: "",
                              "no-caps": "",
                              unelevated: "",
                              type: "submit"
                            }, { ref_for: true }, item.button, {
                              label: item.label,
                              "data-confirm": item.confirm,
                              "data-to-status": item.toStatusId
                            }), null, 16, ["label", "data-confirm", "data-to-status"])
                          ]),
                          _: 2
                        }, 1032, ["action", "fields-initial"]))
                      }), 128))
                    ]))
                  : _createCommentVNode("", true)
              ], 8, ["id", "data-status"]))
            }
        } },
        "m-stylesheet": { hash: "e8119ca2", make: function (Vue) {
            const { openBlock: _openBlock, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createElementBlock("div", {
                type: _ctx.type,
                style: {"display":"none"}
              }, null, 8, ["type"]))
            }
        } },
        "m-subscreens-active": { hash: "49acb545", make: function (Vue) {
            const { resolveDynamicComponent: _resolveDynamicComponent, openBlock: _openBlock, createBlock: _createBlock } = Vue

            return function render(_ctx, _cache) {
              return (_openBlock(), _createBlock(_resolveDynamicComponent(_ctx.activeComponent), { style: {"height":"100%","width":"100%"} }))
            }
        } },
        "m-subscreens-menu": { hash: "7c3c0f81", make: function (Vue) {
            const { renderList: _renderList, Fragment: _Fragment, openBlock: _openBlock, createElementBlock: _createElementBlock, resolveComponent: _resolveComponent, withModifiers: _withModifiers, createBlock: _createBlock, withCtx: _withCtx, createVNode: _createVNode, createCommentVNode: _createCommentVNode } = Vue

            return function render(_ctx, _cache) {
              const _component_q_tab = _resolveComponent("q-tab")
              const _component_q_tabs = _resolveComponent("q-tabs")
              const _component_q_separator = _resolveComponent("q-separator")

              return (_ctx.subscreens.length > 1)
                ? (_openBlock(), _createElementBlock("div", { key: 0 }, [
                    _createVNode(_component_q_tabs, {
                      dense: "",
                      "no-caps": "",
                      align: "left",
                      "narrow-indicator": "",
                      "inline-label": "",
                      class: "text-grey-7",
                      "active-color": "primary",
                      "indicator-color": "primary",
                      "model-value": _ctx.activeTab
                    }, {
                      default: _withCtx(() => [
                        (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.subscreens, (tab) => {
                          return (_openBlock(), _createBlock(_component_q_tab, {
                            key: tab.name,
                            name: tab.name,
                            label: tab.title,
                            disable: tab.disableLink,
                            onClick: _withModifiers($event => (_ctx.goTo(tab.pathWithParams)), ["prevent"])
                          }, null, 8, ["name", "label", "disable", "onClick"]))
                        }), 128))
                      ]),
                      _: 1
                    }, 8, ["model-value"]),
                    _createVNode(_component_q_separator, { class: "q-mb-md" })
                  ]))
                : _createCommentVNode("", true)
            }
        } },
        "m-subscreens-panel": { hash: "3c7bfb94", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, openBlock: _openBlock, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_m_subscreens_tabs = _resolveComponent("m-subscreens-tabs")
              const _component_m_subscreens_active = _resolveComponent("m-subscreens-active")

              return (_openBlock(), _createElementBlock("div", null, [
                _createVNode(_component_m_subscreens_tabs),
                _createVNode(_component_m_subscreens_active)
              ]))
            }
        } },
        "m-subscreens-stack": { hash: "88266d7b", make: function (Vue) {
            const { resolveComponent: _resolveComponent, createVNode: _createVNode, withCtx: _withCtx, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createElementBlock: _createElementBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_q_breadcrumbs_el = _resolveComponent("q-breadcrumbs-el")
              const _component_q_breadcrumbs = _resolveComponent("q-breadcrumbs")
              const _component_m_subscreens_active = _resolveComponent("m-subscreens-active")

              return (_openBlock(), _createElementBlock("div", {
                class: "m-stack",
                id: _ctx.id
              }, [
                (_ctx.stacked)
                  ? (_openBlock(), _createBlock(_component_q_breadcrumbs, {
                      key: 0,
                      class: "m-stack-crumbs q-mb-sm",
                      "active-color": "grey-8"
                    }, {
                      default: _withCtx(() => [
                        _createVNode(_component_q_breadcrumbs_el, {
                          icon: "arrow_back",
                          label: _ctx.base.title,
                          class: "cursor-pointer text-primary",
                          onClick: $event => (_ctx.goTo(_ctx.base))
                        }, null, 8, ["label", "onClick"]),
                        _createVNode(_component_q_breadcrumbs_el, {
                          label: _ctx.active.title,
                          class: "text-weight-medium"
                        }, null, 8, ["label"])
                      ]),
                      _: 1
                    }))
                  : _createCommentVNode("", true),
                _createVNode(_component_m_subscreens_active)
              ], 8, ["id"]))
            }
        } },
        "m-subscreens-wizard": { hash: "b8c5e0fa", make: function (Vue) {
            const { renderList: _renderList, Fragment: _Fragment, openBlock: _openBlock, createElementBlock: _createElementBlock, resolveComponent: _resolveComponent, createBlock: _createBlock, withCtx: _withCtx, createCommentVNode: _createCommentVNode, createVNode: _createVNode, toDisplayString: _toDisplayString, createElementVNode: _createElementVNode } = Vue

            return function render(_ctx, _cache) {
              const _component_q_step = _resolveComponent("q-step")
              const _component_q_stepper = _resolveComponent("q-stepper")
              const _component_m_subscreens_active = _resolveComponent("m-subscreens-active")
              const _component_q_spinner = _resolveComponent("q-spinner")
              const _component_q_inner_loading = _resolveComponent("q-inner-loading")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_space = _resolveComponent("q-space")

              return (_openBlock(), _createElementBlock("div", {
                class: "m-wizard",
                id: _ctx.id,
                "data-step": _ctx.activeName,
                "data-step-form": _ctx.hasForm ? 'true' : 'false',
                "data-forms": _ctx.forms.length
              }, [
                (_ctx.steps.length)
                  ? (_openBlock(), _createBlock(_component_q_stepper, {
                      key: 0,
                      "model-value": _ctx.activeName,
                      flat: "",
                      bordered: "",
                      "header-nav": "",
                      class: "m-wizard-stepper",
                      "active-color": "primary",
                      "done-color": "primary",
                      contracted: _ctx.$q.screen.lt.sm,
                      "onUpdate:modelValue": _ctx.headerTo
                    }, {
                      default: _withCtx(() => [
                        (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.steps, (step, index) => {
                          return (_openBlock(), _createBlock(_component_q_step, {
                            key: step.name,
                            name: step.name,
                            title: step.title,
                            done: index < _ctx.activeIndex,
                            "header-nav": index < _ctx.activeIndex,
                            prefix: step.imageType === 'icon' && step.image ? undefined : String(index + 1),
                            icon: step.imageType === 'icon' && step.image ? step.image : undefined,
                            "data-step": step.name
                          }, null, 8, ["name", "title", "done", "header-nav", "prefix", "icon", "data-step"]))
                        }), 128))
                      ]),
                      _: 1
                    }, 8, ["model-value", "contracted", "onUpdate:modelValue"]))
                  : _createCommentVNode("", true),
                _createElementVNode("div", {
                  ref: "body",
                  class: "m-wizard-body"
                }, [
                  _createVNode(_component_m_subscreens_active),
                  _createVNode(_component_q_inner_loading, {
                    showing: _ctx.submitting,
                    class: "m-wizard-working"
                  }, {
                    default: _withCtx(() => [
                      _createVNode(_component_q_spinner, {
                        color: "primary",
                        size: "40px"
                      }),
                      _createElementVNode("div", {
                        class: "text-caption m-muted q-mt-sm",
                        textContent: _toDisplayString(_ctx.workingLabel)
                      }, null, 8, ["textContent"])
                    ]),
                    _: 1
                  }, 8, ["showing"])
                ], 512),
                (_ctx.steps.length)
                  ? (_openBlock(), _createElementBlock("div", {
                      key: 1,
                      class: "m-wizard-nav row items-center q-gutter-sm"
                    }, [
                      (_ctx.activeIndex > 0)
                        ? (_openBlock(), _createBlock(_component_q_btn, {
                            key: 0,
                            unelevated: "",
                            "no-caps": "",
                            class: "m-btn-tonal m-wizard-back",
                            icon: "arrow_back",
                            label: _ctx.backLabel,
                            disable: _ctx.submitting,
                            onClick: _ctx.back
                          }, null, 8, ["label", "disable", "onClick"]))
                        : _createCommentVNode("", true),
                      _createVNode(_component_q_space),
                      (!_ctx.isLast || _ctx.hasForm)
                        ? (_openBlock(), _createBlock(_component_q_btn, {
                            key: 1,
                            unelevated: "",
                            "no-caps": "",
                            color: "primary",
                            class: "m-wizard-next",
                            "icon-right": _ctx.isLast ? 'check' : 'arrow_forward',
                            label: _ctx.isLast ? _ctx.finishLabel : _ctx.nextLabel,
                            loading: _ctx.submitting,
                            onClick: _ctx.next
                          }, null, 8, ["icon-right", "label", "loading", "onClick"]))
                        : _createCommentVNode("", true)
                    ]))
                  : _createCommentVNode("", true)
              ], 8, ["id", "data-step", "data-step-form", "data-forms"]))
            }
        } },
        "m-text-area": { hash: "c00bcdbf", make: function (Vue) {
            const { resolveComponent: _resolveComponent, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, toDisplayString: _toDisplayString, createElementBlock: _createElementBlock, createVNode: _createVNode, createElementVNode: _createElementVNode, createTextVNode: _createTextVNode, withCtx: _withCtx, mergeProps: _mergeProps } = Vue

            return function render(_ctx, _cache) {
              const _component_m_rich_text = _resolveComponent("m-rich-text")
              const _component_q_editor = _resolveComponent("q-editor")
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_input = _resolveComponent("q-input")

              return (_ctx.editorType === 'rich' || _ctx.editorType === 'rich-light')
                ? (_openBlock(), _createBlock(_component_m_rich_text, {
                    key: 0,
                    light: _ctx.editorType === 'rich-light',
                    "model-value": _ctx.local,
                    "onUpdate:modelValue": _ctx.onInput,
                    name: _ctx.name,
                    id: _ctx.id,
                    label: _ctx.label,
                    form: _ctx.form,
                    disable: _ctx.disable,
                    readonly: _ctx.readonly,
                    tooltip: _ctx.tooltip,
                    rows: _ctx.rows,
                    "mention-url": _ctx.mentionUrl,
                    "mention-parameters": _ctx.mentionParameters
                  }, null, 8, ["light", "model-value", "onUpdate:modelValue", "name", "id", "label", "form", "disable", "readonly", "tooltip", "rows", "mention-url", "mention-parameters"]))
                : (_ctx.editorType === 'html')
                  ? (_openBlock(), _createElementBlock("div", {
                      key: 1,
                      class: "m-text-area-editor"
                    }, [
                      (_ctx.label)
                        ? (_openBlock(), _createElementBlock("div", {
                            key: 0,
                            class: "text-caption text-grey-7"
                          }, _toDisplayString(_ctx.label), 1))
                        : _createCommentVNode("", true),
                      _createVNode(_component_q_editor, {
                        "model-value": _ctx.local,
                        "onUpdate:modelValue": _ctx.onInput,
                        toolbar: _ctx.toolbar,
                        dense: "",
                        "min-height": "12rem",
                        readonly: _ctx.readonly,
                        disable: _ctx.disable
                      }, null, 8, ["model-value", "onUpdate:modelValue", "toolbar", "readonly", "disable"]),
                      _createElementVNode("input", {
                        type: "hidden",
                        name: _ctx.name,
                        value: _ctx.local,
                        form: _ctx.form
                      }, null, 8, ["name", "value", "form"])
                    ]))
                  : (_ctx.editorType === 'md')
                    ? (_openBlock(), _createElementBlock("div", {
                        key: 2,
                        class: "m-text-area-editor"
                      }, [
                        _createVNode(_component_q_input, _mergeProps({
                          type: "textarea",
                          dense: "",
                          outlined: "",
                          "stack-label": "",
                          "hide-bottom-space": "",
                          label: _ctx.label,
                          "model-value": _ctx.local,
                          "onUpdate:modelValue": _ctx.onInput,
                          name: _ctx.name,
                          for: _ctx.id,
                          rows: _ctx.rows,
                          readonly: _ctx.readonly,
                          disable: _ctx.disable,
                          form: _ctx.form,
                          "bg-color": _ctx.bg,
                          autogrow: ""
                        }, _ctx.errorAttrs), {
                          after: _withCtx(() => [
                            _createVNode(_component_q_btn, {
                              flat: "",
                              round: "",
                              dense: "",
                              size: "sm",
                              icon: _ctx.preview ? 'edit' : 'visibility',
                              onClick: _ctx.togglePreview
                            }, {
                              default: _withCtx(() => [
                                _createVNode(_component_q_tooltip, null, {
                                  default: _withCtx(() => [
                                    _createTextVNode("Preview")
                                  ]),
                                  _: 1
                                })
                              ]),
                              _: 1
                            }, 8, ["icon", "onClick"])
                          ]),
                          _: 1
                        }, 16, ["label", "model-value", "onUpdate:modelValue", "name", "for", "rows", "readonly", "disable", "form", "bg-color"]),
                        (_ctx.preview)
                          ? (_openBlock(), _createElementBlock("div", {
                              key: 0,
                              class: "q-pa-sm markdown-preview",
                              innerHTML: _ctx.previewHtml
                            }, null, 8, ["innerHTML"]))
                          : _createCommentVNode("", true)
                      ]))
                    : (_openBlock(), _createBlock(_component_q_input, _mergeProps({
                        key: 3,
                        type: "textarea",
                        dense: "",
                        outlined: "",
                        "stack-label": "",
                        "hide-bottom-space": "",
                        "no-error-icon": "",
                        label: _ctx.label,
                        "model-value": _ctx.local,
                        "onUpdate:modelValue": _ctx.onInput,
                        name: _ctx.name,
                        for: _ctx.id,
                        rows: _ctx.rows,
                        cols: _ctx.cols,
                        readonly: _ctx.readonly,
                        autogrow: _ctx.autogrow,
                        maxlength: _ctx.maxlength,
                        disable: _ctx.disable,
                        form: _ctx.form,
                        "bg-color": _ctx.bg
                      }, _ctx.errorAttrs), {
                        default: _withCtx(() => [
                          (_ctx.tooltip)
                            ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                                default: _withCtx(() => [
                                  _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                                ]),
                                _: 1
                              }))
                            : _createCommentVNode("", true)
                        ]),
                        _: 1
                      }, 16, ["label", "model-value", "onUpdate:modelValue", "name", "for", "rows", "cols", "readonly", "autogrow", "maxlength", "disable", "form", "bg-color"]))
            }
        } },
        "m-text-find": { hash: "77d978ad", make: function (Vue) {
            const { createTextVNode: _createTextVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, createVNode: _createVNode, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, createElementVNode: _createElementVNode, createSlots: _createSlots } = Vue

            return function render(_ctx, _cache) {
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_select = _resolveComponent("q-select")
              const _component_q_toggle = _resolveComponent("q-toggle")
              const _component_q_menu = _resolveComponent("q-menu")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_m_text_line = _resolveComponent("m-text-line")

              return (_openBlock(), _createBlock(_component_m_text_line, {
                label: _ctx.label,
                name: _ctx.name,
                id: _ctx.id,
                size: _ctx.size,
                maxlength: _ctx.maxlength,
                form: _ctx.form,
                tooltip: _ctx.tooltip,
                modelValue: _ctx.fields[_ctx.name],
                "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name]) = $event)
              }, _createSlots({ _: 2 }, [
                (_ctx.hideOptions !== 'true')
                  ? {
                      name: "append",
                      fn: _withCtx(() => [
                        _createVNode(_component_q_btn, {
                          flat: "",
                          round: "",
                          dense: "",
                          size: "sm",
                          icon: "tune",
                          color: _ctx.optionsActive ? 'primary' : 'grey-6'
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_tooltip, null, {
                              default: _withCtx(() => [
                                _createTextVNode("Find Options")
                              ]),
                              _: 1
                            }),
                            _createVNode(_component_q_menu, { class: "m-text-find-options q-pa-sm" }, {
                              default: _withCtx(() => [
                                _createElementVNode("div", { class: "column q-gutter-y-xs" }, [
                                  (_ctx.hideOptions !== 'operator')
                                    ? (_openBlock(), _createBlock(_component_q_select, {
                                        key: 0,
                                        dense: "",
                                        outlined: "",
                                        "options-dense": "",
                                        "emit-value": "",
                                        "map-options": "",
                                        "stack-label": "",
                                        "hide-bottom-space": "",
                                        label: "Operator",
                                        name: _ctx.name + '_op',
                                        modelValue: _ctx.fields[_ctx.name + '_op'],
                                        "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name + '_op']) = $event),
                                        options: _ctx.operators,
                                        form: _ctx.form,
                                        behavior: "menu"
                                      }, null, 8, ["name", "modelValue", "onUpdate:modelValue", "options", "form"]))
                                    : _createCommentVNode("", true),
                                  (_ctx.hideOptions !== 'operator')
                                    ? (_openBlock(), _createBlock(_component_q_toggle, {
                                        key: 1,
                                        dense: "",
                                        size: "sm",
                                        name: _ctx.name + '_not',
                                        label: "Not",
                                        "true-value": "Y",
                                        "false-value": "N",
                                        modelValue: _ctx.fields[_ctx.name + '_not'],
                                        "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name + '_not']) = $event),
                                        form: _ctx.form
                                      }, null, 8, ["name", "modelValue", "onUpdate:modelValue", "form"]))
                                    : _createCommentVNode("", true),
                                  (_ctx.hideOptions !== 'ignore-case')
                                    ? (_openBlock(), _createBlock(_component_q_toggle, {
                                        key: 2,
                                        dense: "",
                                        size: "sm",
                                        name: _ctx.name + '_ic',
                                        label: "Ignore Case",
                                        "true-value": "Y",
                                        "false-value": "N",
                                        modelValue: _ctx.fields[_ctx.name + '_ic'],
                                        "onUpdate:modelValue": $event => ((_ctx.fields[_ctx.name + '_ic']) = $event),
                                        form: _ctx.form
                                      }, null, 8, ["name", "modelValue", "onUpdate:modelValue", "form"]))
                                    : _createCommentVNode("", true)
                                ])
                              ]),
                              _: 1
                            })
                          ]),
                          _: 1
                        }, 8, ["color"])
                      ]),
                      key: "0"
                    }
                  : undefined
              ]), 1032, ["label", "name", "id", "size", "maxlength", "form", "tooltip", "modelValue", "onUpdate:modelValue"]))
            }
        } },
        "m-text-line": { hash: "fdb70647", make: function (Vue) {
            const { toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, renderSlot: _renderSlot, mergeProps: _mergeProps, createSlots: _createSlots } = Vue

            return function render(_ctx, _cache) {
              const _component_q_tooltip = _resolveComponent("q-tooltip")
              const _component_q_input = _resolveComponent("q-input")

              return (_openBlock(), _createBlock(_component_q_input, _mergeProps({
                dense: "",
                outlined: "",
                "stack-label": "",
                "hide-bottom-space": "",
                "no-error-icon": "",
                label: _ctx.label,
                prefix: _ctx.prefix,
                "model-value": _ctx.local,
                "onUpdate:modelValue": _ctx.onInput,
                type: _ctx.type,
                for: _ctx.id,
                name: _ctx.name,
                size: _ctx.size,
                maxlength: _ctx.maxlength,
                form: _ctx.form,
                loading: _ctx.loading,
                rules: _ctx.allRules,
                disable: _ctx.disable,
                "bg-color": _ctx.bg,
                mask: _ctx.mask,
                "fill-mask": _ctx.fillMask,
                "reverse-fill-mask": _ctx.reverseFillMask,
                autocapitalize: "off",
                autocomplete: _ctx.autocomplete
              }, _ctx.errorAttrs), _createSlots({
                default: _withCtx(() => [
                  (_ctx.tooltip)
                    ? (_openBlock(), _createBlock(_component_q_tooltip, { key: 0 }, {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(_ctx.tooltip), 1)
                        ]),
                        _: 1
                      }))
                    : _createCommentVNode("", true)
                ]),
                _: 2
              }, [
                (_ctx.$slots.append)
                  ? {
                      name: "append",
                      fn: _withCtx(() => [
                        _renderSlot(_ctx.$slots, "append")
                      ]),
                      key: "0"
                    }
                  : undefined
              ]), 1040, ["label", "prefix", "model-value", "onUpdate:modelValue", "type", "for", "name", "size", "maxlength", "form", "loading", "rules", "disable", "bg-color", "mask", "fill-mask", "reverse-fill-mask", "autocomplete"]))
            }
        } },
        "m-tree-item": { hash: "f60fcdc3", make: function (Vue) {
            const { normalizeClass: _normalizeClass, openBlock: _openBlock, createElementBlock: _createElementBlock, createCommentVNode: _createCommentVNode, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, createBlock: _createBlock, createElementVNode: _createElementVNode, renderList: _renderList, Fragment: _Fragment, vShow: _vShow, withDirectives: _withDirectives } = Vue

            return function render(_ctx, _cache) {
              const _component_m_link = _resolveComponent("m-link")
              const _component_m_tree_item = _resolveComponent("m-tree-item")

              return (_openBlock(), _createElementBlock("li", {
                id: _ctx.model.id
              }, [
                (_ctx.isFolder)
                  ? (_openBlock(), _createElementBlock("i", {
                      key: 0,
                      onClick: _ctx.toggle,
                      class: _normalizeClass(["fa", {'fa-chevron-right':!_ctx.open, 'fa-chevron-down':_ctx.open}])
                    }, null, 10, ["onClick"]))
                  : (_openBlock(), _createElementBlock("i", {
                      key: 1,
                      class: "fa fa-square-o"
                    })),
                _createTextVNode(),
                _createElementVNode("span", { onClick: _ctx.setSelected }, [
                  (_ctx.linkHref)
                    ? (_openBlock(), _createBlock(_component_m_link, {
                        key: 0,
                        href: _ctx.linkHref,
                        "load-id": _ctx.model.a_attr.loadId,
                        class: _normalizeClass({'text-success':_ctx.selected})
                      }, {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(_ctx.model.text), 1)
                        ]),
                        _: 1
                      }, 8, ["href", "load-id", "class"]))
                    : (_openBlock(), _createElementBlock("span", {
                        key: 1,
                        class: _normalizeClass({'text-success':_ctx.selected})
                      }, _toDisplayString(_ctx.model.text), 3))
                ], 8, ["onClick"]),
                (_ctx.hasChildren)
                  ? _withDirectives((_openBlock(), _createElementBlock("ul", { key: 2 }, [
                      (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.children, (child) => {
                        return (_openBlock(), _createBlock(_component_m_tree_item, {
                          key: child.id,
                          model: child,
                          top: _ctx.top
                        }, null, 8, ["model", "top"]))
                      }), 128))
                    ], 512)), [
                      [_vShow, _ctx.open]
                    ])
                  : _createCommentVNode("", true)
              ], 8, ["id"]))
            }
        } },
        "m-tree-top": { hash: "14ff4400", make: function (Vue) {
            const { renderList: _renderList, Fragment: _Fragment, openBlock: _openBlock, createElementBlock: _createElementBlock, resolveComponent: _resolveComponent, createBlock: _createBlock } = Vue

            return function render(_ctx, _cache) {
              const _component_m_tree_item = _resolveComponent("m-tree-item")

              return (_openBlock(), _createElementBlock("ul", {
                id: _ctx.id,
                class: "tree-list"
              }, [
                (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.itemList, (model) => {
                  return (_openBlock(), _createBlock(_component_m_tree_item, {
                    key: model.id,
                    model: model,
                    top: _ctx.top
                  }, null, 8, ["model", "top"]))
                }), 128))
              ], 8, ["id"]))
            }
        } },
        "m-user-status-dialog": { hash: "8b5ac844", make: function (Vue) {
            const { toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, createVNode: _createVNode, renderList: _renderList, Fragment: _Fragment, openBlock: _openBlock, createElementBlock: _createElementBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, withKeys: _withKeys, createElementVNode: _createElementVNode, resolveDirective: _resolveDirective, withDirectives: _withDirectives } = Vue

            return function render(_ctx, _cache) {
              const _component_q_card_section = _resolveComponent("q-card-section")
              const _component_q_item_section = _resolveComponent("q-item-section")
              const _component_q_item = _resolveComponent("q-item")
              const _component_q_list = _resolveComponent("q-list")
              const _component_q_input = _resolveComponent("q-input")
              const _component_q_select = _resolveComponent("q-select")
              const _component_q_btn = _resolveComponent("q-btn")
              const _component_q_card_actions = _resolveComponent("q-card-actions")
              const _component_q_card = _resolveComponent("q-card")
              const _component_q_dialog = _resolveComponent("q-dialog")
              const _directive_close_popup = _resolveDirective("close-popup")

              return (_openBlock(), _createBlock(_component_q_dialog, {
                modelValue: _ctx.open,
                "onUpdate:modelValue": $event => ((_ctx.open) = $event)
              }, {
                default: _withCtx(() => [
                  _createVNode(_component_q_card, { style: {"min-width":"320px","max-width":"96vw"} }, {
                    default: _withCtx(() => [
                      _createVNode(_component_q_card_section, { class: "text-subtitle1" }, {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(_ctx.labels.setStatus), 1)
                        ]),
                        _: 1
                      }),
                      _createVNode(_component_q_card_section, { class: "q-pt-none q-gutter-y-sm" }, {
                        default: _withCtx(() => [
                          (_ctx.presets.length)
                            ? (_openBlock(), _createBlock(_component_q_list, {
                                key: 0,
                                dense: "",
                                class: "m-status-presets"
                              }, {
                                default: _withCtx(() => [
                                  (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.presets, (preset, index) => {
                                    return (_openBlock(), _createBlock(_component_q_item, {
                                      key: index,
                                      clickable: "",
                                      dense: "",
                                      "data-preset": index,
                                      onClick: $event => (_ctx.usePreset(preset))
                                    }, {
                                      default: _withCtx(() => [
                                        _createVNode(_component_q_item_section, {
                                          avatar: "",
                                          style: {"min-width":"28px"}
                                        }, {
                                          default: _withCtx(() => [
                                            _createTextVNode(_toDisplayString(preset.emoji), 1)
                                          ]),
                                          _: 2
                                        }, 1024),
                                        _createVNode(_component_q_item_section, null, {
                                          default: _withCtx(() => [
                                            _createTextVNode(_toDisplayString(preset.text), 1)
                                          ]),
                                          _: 2
                                        }, 1024),
                                        _createVNode(_component_q_item_section, {
                                          side: "",
                                          class: "text-caption"
                                        }, {
                                          default: _withCtx(() => [
                                            _createTextVNode(_toDisplayString(_ctx.clearLabel(preset.clearAfter)), 1)
                                          ]),
                                          _: 2
                                        }, 1024)
                                      ]),
                                      _: 2
                                    }, 1032, ["data-preset", "onClick"]))
                                  }), 128))
                                ]),
                                _: 1
                              }))
                            : _createCommentVNode("", true),
                          _createElementVNode("div", { class: "row no-wrap q-gutter-x-sm" }, [
                            _createVNode(_component_q_input, {
                              modelValue: _ctx.emoji,
                              "onUpdate:modelValue": $event => ((_ctx.emoji) = $event),
                              dense: "",
                              outlined: "",
                              "stack-label": "",
                              label: _ctx.labels.emoji,
                              maxlength: "16",
                              style: {"width":"84px"},
                              name: "emoji"
                            }, null, 8, ["modelValue", "onUpdate:modelValue", "label"]),
                            _createVNode(_component_q_input, {
                              modelValue: _ctx.text,
                              "onUpdate:modelValue": $event => ((_ctx.text) = $event),
                              dense: "",
                              outlined: "",
                              "stack-label": "",
                              autofocus: "",
                              class: "col",
                              label: _ctx.labels.statusText,
                              maxlength: "100",
                              name: "statusText",
                              onKeyup: _withKeys(_ctx.save, ["enter"])
                            }, null, 8, ["modelValue", "onUpdate:modelValue", "label", "onKeyup"])
                          ]),
                          _createVNode(_component_q_select, {
                            modelValue: _ctx.clear,
                            "onUpdate:modelValue": $event => ((_ctx.clear) = $event),
                            options: _ctx.clearOptions,
                            "emit-value": "",
                            "map-options": "",
                            dense: "",
                            outlined: "",
                            "stack-label": "",
                            label: _ctx.labels.clearAfter,
                            name: "clearAt"
                          }, null, 8, ["modelValue", "onUpdate:modelValue", "options", "label"])
                        ]),
                        _: 1
                      }),
                      _createVNode(_component_q_card_actions, { align: "right" }, {
                        default: _withCtx(() => [
                          _withDirectives(_createVNode(_component_q_btn, {
                            flat: "",
                            "no-caps": "",
                            label: _ctx.labels.cancel
                          }, null, 8, ["label"]), [
                            [_directive_close_popup]
                          ]),
                          _createVNode(_component_q_btn, {
                            unelevated: "",
                            "no-caps": "",
                            color: "primary",
                            label: _ctx.labels.save,
                            loading: _ctx.saving,
                            id: "m-status-save",
                            onClick: _ctx.save
                          }, null, 8, ["label", "loading", "onClick"])
                        ]),
                        _: 1
                      })
                    ]),
                    _: 1
                  })
                ]),
                _: 1
              }, 8, ["modelValue", "onUpdate:modelValue"]))
            }
        } },
        "m-user-status-menu": { hash: "8683de21", make: function (Vue) {
            const { normalizeClass: _normalizeClass, createElementVNode: _createElementVNode, resolveComponent: _resolveComponent, withCtx: _withCtx, createVNode: _createVNode, toDisplayString: _toDisplayString, createTextVNode: _createTextVNode, openBlock: _openBlock, createBlock: _createBlock, createCommentVNode: _createCommentVNode, renderList: _renderList, Fragment: _Fragment, createElementBlock: _createElementBlock, resolveDirective: _resolveDirective, withDirectives: _withDirectives } = Vue

            return function render(_ctx, _cache) {
              const _component_q_item_section = _resolveComponent("q-item-section")
              const _component_q_item_label = _resolveComponent("q-item-label")
              const _component_q_item = _resolveComponent("q-item")
              const _component_q_icon = _resolveComponent("q-icon")
              const _directive_close_popup = _resolveDirective("close-popup")

              return (_openBlock(), _createElementBlock("div", { class: "m-user-status" }, [
                _createVNode(_component_q_item, {
                  id: "m-status-current",
                  dense: ""
                }, {
                  default: _withCtx(() => [
                    _createVNode(_component_q_item_section, {
                      avatar: "",
                      class: "m-status-avatar"
                    }, {
                      default: _withCtx(() => [
                        _createElementVNode("span", {
                          class: _normalizeClass(["m-status-dot", 'm-presence-' + _ctx.currentDot])
                        }, null, 2)
                      ]),
                      _: 1
                    }),
                    _createVNode(_component_q_item_section, null, {
                      default: _withCtx(() => [
                        _createVNode(_component_q_item_label, null, {
                          default: _withCtx(() => [
                            _createTextVNode(_toDisplayString(_ctx.currentLabel), 1)
                          ]),
                          _: 1
                        }),
                        (_ctx.me.statusText || _ctx.me.emoji)
                          ? (_openBlock(), _createBlock(_component_q_item_label, {
                              key: 0,
                              caption: "",
                              class: "m-status-text"
                            }, {
                              default: _withCtx(() => [
                                _createTextVNode(_toDisplayString(_ctx.me.emoji ? _ctx.me.emoji + ' ' : '') + _toDisplayString(_ctx.me.statusText), 1)
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
                (_openBlock(true), _createElementBlock(_Fragment, null, _renderList(_ctx.options, (option) => {
                  return _withDirectives((_openBlock(), _createBlock(_component_q_item, {
                    key: option.value,
                    dense: "",
                    clickable: "",
                    active: _ctx.current === option.value,
                    "data-status": option.value,
                    onClick: $event => (_ctx.choose(option.value))
                  }, {
                    default: _withCtx(() => [
                      _createVNode(_component_q_item_section, {
                        avatar: "",
                        class: "m-status-avatar"
                      }, {
                        default: _withCtx(() => [
                          _createElementVNode("span", {
                            class: _normalizeClass(["m-status-dot", 'm-presence-' + option.dot])
                          }, null, 2)
                        ]),
                        _: 2
                      }, 1024),
                      _createVNode(_component_q_item_section, null, {
                        default: _withCtx(() => [
                          _createTextVNode(_toDisplayString(option.label), 1)
                        ]),
                        _: 2
                      }, 1024)
                    ]),
                    _: 2
                  }, 1032, ["active", "data-status", "onClick"])), [
                    [_directive_close_popup]
                  ])
                }), 128)),
                _withDirectives((_openBlock(), _createBlock(_component_q_item, {
                  dense: "",
                  clickable: "",
                  "data-status": "message",
                  onClick: _ctx.openDialog
                }, {
                  default: _withCtx(() => [
                    _createVNode(_component_q_item_section, {
                      avatar: "",
                      class: "m-status-avatar"
                    }, {
                      default: _withCtx(() => [
                        _createVNode(_component_q_icon, {
                          name: "sym_o_edit",
                          size: "xs"
                        })
                      ]),
                      _: 1
                    }),
                    _createVNode(_component_q_item_section, null, {
                      default: _withCtx(() => [
                        _createTextVNode(_toDisplayString(_ctx.labels.setStatus), 1)
                      ]),
                      _: 1
                    })
                  ]),
                  _: 1
                }, 8, ["onClick"])), [
                  [_directive_close_popup]
                ]),
                (_ctx.me.statusText || _ctx.me.emoji)
                  ? _withDirectives((_openBlock(), _createBlock(_component_q_item, {
                      key: 0,
                      dense: "",
                      clickable: "",
                      "data-status": "clear",
                      onClick: _ctx.clearText
                    }, {
                      default: _withCtx(() => [
                        _createVNode(_component_q_item_section, {
                          avatar: "",
                          class: "m-status-avatar"
                        }, {
                          default: _withCtx(() => [
                            _createVNode(_component_q_icon, {
                              name: "sym_o_close",
                              size: "xs"
                            })
                          ]),
                          _: 1
                        }),
                        _createVNode(_component_q_item_section, null, {
                          default: _withCtx(() => [
                            _createTextVNode(_toDisplayString(_ctx.labels.clearStatus), 1)
                          ]),
                          _: 1
                        })
                      ]),
                      _: 1
                    }, 8, ["onClick"])), [
                      [_directive_close_popup]
                    ])
                  : _createCommentVNode("", true)
              ]))
            }
        } }
    };
})(globalThis.moqui);
