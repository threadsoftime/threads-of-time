--[[
ItemRef.lua — mod-warforged FrameXML override for chat-link tooltips (v1.0)

Same pattern as GameTooltip.lua, registered against ItemRefTooltip — this is
the tooltip that appears when a player shift-clicks an item link in chat.

Helpers are duplicated rather than shared via a 3rd file because the MPQ load
order for FrameXML overrides in 3.3.5a is fragile; keeping each override self-
contained avoids "WF_STAT_BUMPS is nil because the helper file loaded second"
surprises. If a v1.1 shared module makes sense, dedupe via a single
WarforgedHelpers.lua loaded ahead of both via XML.
]]--

local WF_ENCH_MIN, WF_ENCH_MAX = 70001, 70063
local WF_TAG_COLOR = "|cffff8000"
local WF_ILVL_BUMP = 5

local function IsWarforgedEnchant(id)
    id = tonumber(id) or 0
    return id >= WF_ENCH_MIN and id <= WF_ENCH_MAX
end

local function GetBonusEnchantFromLink(itemLink)
    if not itemLink then return 0 end
    local parts = { strsplit(":", itemLink) }
    return tonumber(parts[8] or "0") or 0
end

local function FormatTooltipForWarforged(tooltip, bonusEnchantId)
    local name = tooltip:GetName()
    local numLines = tooltip:NumLines()

    for i = 1, numLines do
        local line = _G[name .. "TextLeft" .. i]
        if line then
            local text = line:GetText() or ""
            local ilvl = text:match("^Item Level (%d+)$")
            if ilvl then
                line:SetText("Item Level " .. (tonumber(ilvl) + WF_ILVL_BUMP))
            end
        end
    end

    for i = 1, numLines do
        local line = _G[name .. "TextLeft" .. i]
        if line then
            local text = line:GetText() or ""
            if text:match("^Warforged:") then
                line:SetText("")
            end
        end
    end

    tooltip:AddLine(WF_TAG_COLOR .. "Warforged|r")
    tooltip:Show()
end

ItemRefTooltip:HookScript("OnTooltipSetItem", function(self)
    local _, itemLink = self:GetItem()
    if not itemLink then return end
    local bonusId = GetBonusEnchantFromLink(itemLink)
    if IsWarforgedEnchant(bonusId) then
        FormatTooltipForWarforged(self, bonusId)
    end
end)
