(function() {
    var found_xafind = ec.entity.find("moqui.test.TestEntity").orderBy("testId").condition("testId", seedId);
    var found = found_xafind.list();
    var oneRow = ec.entity.find("moqui.test.TestEntity").condition("testId", seedId).one();
    return;
})()
