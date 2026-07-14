export default defineBackground(() => {
  browser.runtime.onInstalled.addListener(() => {
    console.info("Agent Ferry extension installed");
  });
});
