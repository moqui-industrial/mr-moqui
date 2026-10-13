(function() {
    if (ready) {
    statusId = orderStatus;
    } else {
    statusId = ec.resource.expand("N");
    }
    while (ready) {
    n = n + 1;
    }
    if (rows != null) {
        var row_index = 0;
        var row_len = (typeof rows.length === "number") ? rows.length : 0;
        while (row_index < row_len) {
            var row = (typeof rows.get === "function") ? rows.get(row_index) : rows[row_index];
            row_has_next = (row_index + 1) < row_len;
    n = row;
            row_index = row_index + 1;
        }
    }
    return;
})()
