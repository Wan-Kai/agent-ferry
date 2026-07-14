import React from "react";
import ReactDOM from "react-dom/client";
import "./style.css";

function App() {
  return (
    <main>
      <p className="eyebrow">AGENT FERRY</p>
      <h1>把当前页面交给你的 Agent</h1>
      <p className="description">项目正在搭建中，页面提取和工作区选择将在下一个里程碑实现。</p>
      <button type="button" disabled>
        准备交接
      </button>
    </main>
  );
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
