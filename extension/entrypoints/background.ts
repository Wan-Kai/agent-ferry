export default defineBackground(() => {
  browser.runtime.onInstalled.addListener((details) => {
    if (details.reason !== "install") return;
    void browser.tabs.create({ url: browser.runtime.getURL("/onboarding.html") });
  });
});
