(function() {
    if (true) {
        var call_service_result = ec.service.sync().name("org.moqui.example.ExampleServices.do#Thing").disableAuthz().requireNewTransaction(true).transactionTimeout(30).useTransactionCache(false).parameters(context).parameter("orderId", orderId).call();
        if (result != null) { if (call_service_result) Object.assign(result, call_service_result); } else { result = call_service_result; }
        if (ec.message.hasError()) return;
    }
    return;
})()
