(function() {
    var row = ec.entity.makeValue("moqui.test.TestEntity");
    row.setFields(seedMap, true, null, null);
    row.create();
    row.update();
    row.delete();
    return;
})()
