/* This software is in the public domain under CC0 1.0 Universal plus a Grant of Patent License. */

/*
 * The public shell of the anonymous token routes (/s/token, /u/token, /call/token; PublicShell.xml): a Vue and Quasar
 * application with the m-* components of WebrootVue.qvt.js (loaded before this, for its components only) that shows one
 * screen, the route's, with no header, drawers or login. It takes its settings from
 * <script type="application/json" id="moqui-public-config">: route {name, basePath, title, renderModes}, context (what the
 * route's token check answered), the locale and the theme's Quasar configuration. The screen loads as basePath.qvt (or
 * .qvue, .qjs, by its render modes) and its transitions are under basePath. The root offers the parts of m-qapps-root a
 * screen and the m-* components use (setUrl, currentParameters, currentLinkPath, basePath, ...); a link outside the route
 * leaves the page.
 */
(function (root) {
    "use strict";

    const moqui = root.moqui;
    const qvt = moqui && moqui.qvt;

    const ROOT_TEMPLATE = '<q-layout view="hHh lpR fFf" class="m-public-shell"><q-page-container><q-page class="q-pa-md">' +
        '<div class="m-public-page" style="max-width:1100px;margin:0 auto;">' +
        '<component v-if="activeComponent" :is="activeComponent"></component>' +
        '<div v-else-if="failed" class="text-body1 q-pa-lg text-center">{{labels.notFound}}</div>' +
        '<div v-else class="row justify-center q-pa-xl"><q-spinner size="2em"></q-spinner></div>' +
        '</div></q-page></q-page-container></q-layout>';

    const queryOf = (search) => {
        const out = {};
        new root.URLSearchParams(search || "").forEach((value, key) => { out[key] = value; });
        return out;
    };

    // The root's options: the route's screen as the one active component, and the parts of the /qapps root the components ask
    // for. Exposed as moqui.publicRootOptions for a test.
    const publicRootOptions = (config) => {
        const route = config.route || {};
        const basePath = route.basePath || "";
        return {
            data() {
                return {
                    activeComponent: null, failed: false, loading: 0, config: config, labels: config.labels || {}, context: config.context || {},
                    basePath: basePath, linkBasePath: basePath, appRootPath: config.appRootPath || "", currentPath: basePath,
                    currentPathList: [], currentParameters: queryOf(root.location ? root.location.search : ""), activeSubscreens: [],
                    navMenuList: [], moquiSessionToken: config.sessionToken || "", userId: null, formsVersion: 0, notifyHistoryList: []
                };
            },
            computed: {
                currentLinkPath() { return this.basePath; }
            },
            methods: {
                // a path under the route stays here (its parameters reload the screen); any other URL leaves the page
                setUrl(url) {
                    if (url == null) return;
                    const href = typeof url === "string" ? url : moqui.makeHref(url);
                    const parsed = moqui.parseHref(href);
                    if (parsed.path === this.basePath || parsed.path === "") {
                        this.currentParameters = parsed.query || {};
                        if (root.history && root.history.replaceState) root.history.replaceState(null, "", href);
                        this.load();
                    } else root.location.assign(href);
                },
                setParameters(parameters) { this.currentParameters = Object.assign({}, this.currentParameters, parameters || {}); },
                getLinkPath(path) { return path; },
                getRoute() { return { path: this.basePath, matched: [{ path: this.basePath }], params: {}, query: this.currentParameters }; },
                addNotify(message, type) { this.notifyHistoryList.unshift({ message, type, time: Date.now() }); },
                reLoginCheckShow() { root.location.reload(); },
                setPreference() {},
                setScreenContext() {},
                removeScreenContext() {},
                addSubscreen() {},
                addContainer() {},
                removeContainer() {},
                hideContainer() {},
                loadContainer() {},
                reloadContainer() {},
                load() {
                    this.loading++;
                    const search = Object.keys(this.currentParameters).map((key) => encodeURIComponent(key) + "=" +
                        encodeURIComponent(this.currentParameters[key])).join("&");
                    moqui.loadComponent({ path: this.basePath, search: search, renderModes: route.renderModes || [] }, (comp) => {
                        this.loading = Math.max(0, this.loading - 1);
                        if (!comp || comp === moqui.NotFound) { this.failed = true; this.activeComponent = null; return; }
                        this.failed = false;
                        this.activeComponent = comp;
                    }, "public-screen-root");
                }
            },
            created() {
                moqui.sessionToken = this.moquiSessionToken;
                moqui.appRootPath = this.appRootPath;
            },
            mounted() {
                const container = root.document.getElementById("public-root");
                if (container) container.style.display = "";
                this.load();
            }
        };
    };
    if (moqui) moqui.publicRootOptions = publicRootOptions;

    const start = () => {
        const doc = root.document;
        const config = JSON.parse(doc.getElementById("moqui-public-config").textContent);
        if (qvt) {
            qvt.applyRenders();
            qvt.config = config;
        }
        moqui.config = config;
        if (moqui.setNumberSeparators) moqui.setNumberSeparators(config.locale);
        if (moqui.l10n) {
            moqui.l10n.setLocale(config.locale);
            moqui.l10n.setTimeZone(config.timeZone);
        }
        const options = publicRootOptions(config);
        options.template = ROOT_TEMPLATE;
        const app = root.Vue.createApp(options);
        const quasarOptions = { config: Object.assign({ notify: { progress: true, closeBtn: "X", position: "top-right" } }, config.quasar),
            plugins: { Notify: root.Quasar.Notify, LoadingBar: root.Quasar.LoadingBar } };
        const langKey = String(config.quasarLang || "enUS").replace(/-([a-zA-Z])/g, (m, ch) => ch.toUpperCase()).replace("-", "");
        if (root.Quasar.Lang && root.Quasar.Lang[langKey]) quasarOptions.lang = root.Quasar.Lang[langKey];
        if (root.Quasar.IconSet && root.Quasar.IconSet.materialSymbolsOutlined) quasarOptions.iconSet = root.Quasar.IconSet.materialSymbolsOutlined;
        app.use(root.Quasar, quasarOptions);
        if (qvt) Object.keys(qvt.components).forEach((name) => { if (name !== "m-qapps-root") app.component(name, qvt.components[name]); });
        app.config.globalProperties.moqui = moqui;
        app.config.globalProperties.window = root;
        if (qvt && qvt.makeRouter) app.config.globalProperties.$router = qvt.makeRouter();
        Object.defineProperty(app.config.globalProperties, "$route", { get: () => (moqui.webrootVue ? moqui.webrootVue.getRoute() : { matched: [], params: {} }) });
        app.config.errorHandler = (err, instance, info) => { console.error("Vue error (" + info + ")", err); };
        moqui.webrootVue = app.mount("#public-root");
        return moqui.webrootVue;
    };
    if (moqui) moqui.startPublic = start;

    if (root.document && root.document.getElementById && root.document.getElementById("moqui-public-config") && root.Vue && root.Quasar && moqui) start();
})(globalThis);
