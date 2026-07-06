/* =========================================================================
   AI-OS Kernel Dashboard — Client-side Application Logic
   ========================================================================= */

(function () {
    "use strict";

    // -----------------------------------------------------------------------
    // Tab navigation
    // -----------------------------------------------------------------------

    function switchTab(tabId) {
        // Update nav items
        document.querySelectorAll(".nav-item").forEach(function (el) {
            el.classList.toggle("active", el.dataset.tab === tabId);
        });

        // Update tab panels
        document.querySelectorAll(".tab-panel").forEach(function (el) {
            el.classList.toggle("active", el.id === tabId + "-tab");
        });

        // Update URL hash without scrolling
        history.replaceState(null, "", "#" + tabId);
    }

    // Delegate nav clicks
    document.addEventListener("click", function (e) {
        var navItem = e.target.closest(".nav-item");
        if (navItem && navItem.dataset.tab) {
            e.preventDefault();
            var tabId = navItem.dataset.tab;

            // If clicking already-active tab, just scroll to top
            if (navItem.classList.contains("active")) {
                document.querySelector(".main-content").scrollTop = 0;
                return;
            }

            switchTab(tabId);

            // Scroll main content to top on tab switch
            document.querySelector(".main-content").scrollTop = 0;
        }
    });

    // Restore tab from URL hash on load
    function restoreTab() {
        var hash = window.location.hash.replace("#", "");
        var validTabs = ["overview", "timeline", "objectives", "audit", "metrics"];
        if (hash && validTabs.indexOf(hash) !== -1) {
            switchTab(hash);
        }
    }

    // -----------------------------------------------------------------------
    // Status badge CSS class resolver
    // -----------------------------------------------------------------------

    window.statusBadge = function (status) {
        if (!status) return "badge-gray";
        var s = status.toLowerCase();
        if (s === "done") return "badge-green";
        if (s === "completed") return "badge-green";
        if (s === "active" || s === "discovered" || s === "ready") return "badge-blue";
        if (s === "executing" || s === "running" || s === "in_progress") return "badge-orange";
        if (s === "failed") return "badge-red";
        if (s === "abandoned" || s === "cancelled") return "badge-gray";
        if (s === "pending") return "badge-cyan";
        return "badge-gray";
    };

    // -----------------------------------------------------------------------
    // Format ISO timestamp to local time string
    // -----------------------------------------------------------------------

    window.formatTime = function (isoStr) {
        if (!isoStr) return "-";
        try {
            var d = new Date(isoStr);
            if (isNaN(d.getTime())) return isoStr;
            return d.toLocaleString();
        } catch (_) {
            return isoStr;
        }
    };

    // -----------------------------------------------------------------------
    // Truncate hash for display
    // -----------------------------------------------------------------------

    window.truncHash = function (hash, len) {
        len = len || 12;
        if (!hash || hash.length <= len) return hash || "-";
        return hash.substring(0, len) + "...";
    };

    // -----------------------------------------------------------------------
    // Shorten event ID for display
    // -----------------------------------------------------------------------

    window.shortId = function (id, len) {
        len = len || 16;
        if (!id || id.length <= len) return id || "-";
        return id.substring(0, len) + "...";
    };

    // -----------------------------------------------------------------------
    // JSON pretty-print for event payload display
    // -----------------------------------------------------------------------

    window.jsonPreview = function (obj) {
        if (!obj) return "-";
        try {
            var str = typeof obj === "string" ? obj : JSON.stringify(obj);
            if (str.length > 120) {
                return str.substring(0, 120) + "...";
            }
            return str;
        } catch (_) {
            return String(obj);
        }
    };

    // -----------------------------------------------------------------------
    // Auto-refresh for the overview tab (every 10 seconds)
    // -----------------------------------------------------------------------

    var refreshInterval = null;

    function startAutoRefresh() {
        stopAutoRefresh();
        refreshInterval = setInterval(function () {
            var overviewPanel = document.getElementById("overview-tab");
            if (overviewPanel && overviewPanel.classList.contains("active")) {
                htmx.trigger("#overview-content", "refresh");
            }
        }, 10000);
    }

    function stopAutoRefresh() {
        if (refreshInterval) {
            clearInterval(refreshInterval);
            refreshInterval = null;
        }
    }

    // Monitor which tab is active for auto-refresh
    var observer = new MutationObserver(function () {
        var overviewPanel = document.getElementById("overview-tab");
        if (overviewPanel && overviewPanel.classList.contains("active")) {
            startAutoRefresh();
        } else {
            stopAutoRefresh();
        }
    });

    // -----------------------------------------------------------------------
    // Initialize on DOM ready
    // -----------------------------------------------------------------------

    document.addEventListener("DOMContentLoaded", function () {
        restoreTab();
        startAutoRefresh();

        var overviewPanel = document.getElementById("overview-tab");
        if (overviewPanel) {
            observer.observe(overviewPanel, {
                attributes: true,
                attributeFilter: ["class"],
            });
        }
    });

})();
