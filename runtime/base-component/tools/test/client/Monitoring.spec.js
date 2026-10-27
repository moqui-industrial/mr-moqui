/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */
spec({})
function monitoringPage() {
    const page=browser.newPage({user:"john.doe"})
    // Commons returns404 for an absent user photo; the avatar deliberately displays initials (GalleryShots uses this contract too).
    page.allowProblems(/response: 404 Not Found .*\/apps\/Commons\/Settings\/Account\/userPhoto\?userId=EX_JOHN_DOE$/)
    return page
}
feature("a shipped dashboard renders through the report engine in the qvt shell", {
    expect() {
        const page = monitoringPage()
        page.goto("/qapps/dashboards/Dashboard?definitionId=monitor-overview&from=now-1d&thru=now").status == 200
        page.waitFor(() => !!document.querySelector(".m-report-dashboard"))
        page.text().includes("Monitor overview")
        page.text().includes("HTTP requests")
        !/Error rendering|Exception in render|Internal Server Error/.test(page.text())
    },
})
feature("shipped definitions explain the copy workflow and expose version history", {
    expect() {
        const page = monitoringPage()
        page.goto("/apps/dashboards/Definition?definitionId=monitor-overview").status == 200
        page.text().includes("Save a copy to edit it")
        page.text().includes("Save as copy")
        page.text().includes("Export with references")
    },
})
feature("the authorized log search is usable without the qvt shell", {
    expect() {
        const page = monitoringPage()
        page.goto("/apps/monitor/Logs?from=now-1h&thru=now").status == 200
        page.text().includes("Queries run with your dataset permissions")
        page.text().includes("Query")
        !/Error rendering|Exception in render|Internal Server Error/.test(page.text())
    },
})
feature("Explore renders the compiler-free query editor and applies a typed filter through its form", {
    expect() {
        const page=monitoringPage()
        page.goto("/qapps/dashboards/Explore?datasetId=otel_logs&from=now-1h&thru=now").status==200
        page.waitFor(()=>!!document.querySelector('input[aria-label="Search query"]'))
        page.evaluate(()=>{
            const input=document.querySelector('input[aria-label="Search query"]'),editor=input.closest('section')
            const field=Array.from(editor.querySelectorAll('label')).find(label=>label.textContent.startsWith('Field ')).querySelector('select')
            field.value='severity_text';field.dispatchEvent(new Event('change',{bubbles:true}))
            const value=Array.from(editor.querySelectorAll('label')).find(label=>label.textContent.startsWith('Value ')).querySelector('input')
            value.value='ERROR';value.dispatchEvent(new Event('input',{bubbles:true}))
            Array.from(editor.querySelectorAll('button')).find(button=>button.textContent==='Add filter').click();return true
        })
        page.waitFor(()=>document.querySelector('[aria-label="Query filters"]').textContent.includes('severity_text'))
        page.evaluate(()=>{
            const editor=document.querySelector('input[aria-label="Search query"]').closest('section')
            Array.from(editor.querySelectorAll('button')).find(button=>button.textContent==='Negate').click()
            Array.from(editor.querySelectorAll('button')).find(button=>button.textContent==='Disable').click();return true
        })
        page.waitFor(()=>document.querySelector('[aria-label="Query filters"]').textContent.includes('(disabled)'))
        page.evaluate(()=>{document.querySelector('input[aria-label="Search query"]').closest('form').requestSubmit();return true})
        page.waitFor(()=>!!document.querySelector('input[aria-label="Search query"]'))
        !/Error rendering|Exception in render|Internal Server Error/.test(page.text())
    },
})
feature("new panel definitions expose searchable options in the runtime-only browser", {
    expect() {
        const page=monitoringPage()
        page.goto("/qapps/dashboards/Definition?definitionType=panel").status==200
        page.waitFor(()=>!!document.querySelector('input[placeholder="Search panel options"]'))
        page.evaluate(()=>{const input=document.querySelector('input[placeholder="Search panel options"]');input.value='Decimal';input.dispatchEvent(new Event('input',{bubbles:true}));return true})
        page.waitFor(()=>Array.from(document.querySelectorAll('label')).some(label=>label.textContent==='Decimal places'))
        !/Error rendering|Exception in render|Internal Server Error/.test(page.text())
    },
})
feature("explicit dashboard navigation records recent dashboards without writing during a view", {
    expect() {
        const page=monitoringPage()
        page.goto("/qapps/dashboards/Home").status==200
        page.waitFor(()=>Array.from(document.querySelectorAll('a,button')).some(link=>link.textContent.trim()==='Monitor overview'))
        page.evaluate(()=>{const link=Array.from(document.querySelectorAll('a,button')).find(link=>link.textContent.trim()==='Monitor overview');link.click();return true})
        page.waitFor(()=>!!document.querySelector('.m-report-dashboard'))
        page.goto("/apps/dashboards/Home").status==200
        const recent=page.text().split('Recent')[1].split('All definitions')[0]
        recent.includes('Monitor overview')
    },
})
