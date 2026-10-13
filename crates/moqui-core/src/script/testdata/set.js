(function() {
    statusId = orderStatus;
    label = ec.resource.expand("Hello ${name}");
    var _temp_internal = orderStatus;
    if (_temp_internal != null && _temp_internal !== "") code = _temp_internal;
    fallback = ((orderStatus == null || orderStatus === "") ? ec.resource.expand("Open") : (orderStatus));
    return;
})()
