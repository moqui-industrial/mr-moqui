// Fails on purpose, for the report and exit status tests: it is copied into a test directory by the test that uses it.
spec({ stepwise: true })

feature("a false condition", {
    when() { return { priceMap: { price: toDecimal("16.98") } } },
    then({ priceMap }) {
        priceMap.price == 16.99
    },
})

feature("skipped after the failure", {
    expect() { true },
})
