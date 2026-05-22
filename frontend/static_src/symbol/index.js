// Symbol detail page: the lightweight-charts chart + range selector, and the
// annual / quarterly financials toggle.
import "./styles/symbol.scss";
import { initChart } from "./scripts/chart.js";
import { initFundamentals } from "./scripts/fundamentals.js";
import { initGrowth } from "./scripts/growth.js";

initChart();
initFundamentals();
initGrowth();
