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
        var _row_list = rows;
        var row_index = 0;
        var row_len = (typeof _row_list.length === "number") ? _row_list.length : 0;
        for (; row_index < row_len; row_index = row_index + 1) {
            row = (typeof _row_list.get === "function") ? _row_list.get(row_index) : _row_list[row_index];
            row_has_next = (row_index + 1) < row_len;
            n = row;
        }
    }
    return;
})()
