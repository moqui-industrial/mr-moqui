(function() {
    ec.message.addError(ec.resource.expand("failed ${orderId}"));
    ec.message.addMessage(ec.resource.expand("done"), "info");
    if (ec.message.hasError()) return;
    ec.message.addMessage(ec.resource.expand("stop"), "info");
    return;
    return;
})()
