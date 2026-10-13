if (name) {
    sri.write($csv(ec.resource.expand("Cond ${name}")));
}
sri.write($csv(ec.resource.expand("Mapped ${v}", "", {v: 2})));
