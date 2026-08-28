using System;
using System.Collections.Generic;

namespace Aurorix.Windows.Shell;

/// <summary>
/// Presentation-only route metadata. Domain state and command dispatch stay
/// behind the Core facade and are intentionally absent from this type.
/// </summary>
public sealed record ShellRoute(string Key, string Title, string Description);

public static class ShellNavigation
{
    private static readonly IReadOnlyDictionary<string, ShellRoute> Routes =
        new Dictionary<string, ShellRoute>(StringComparer.OrdinalIgnoreCase)
        {
            ["home"] = new("home", "首页", "本地首页内容将在 Home facade 接入后呈现。"),
            ["library"] = new("library", "音乐库", "音乐库宿主已就绪，目录数据由 Core facade 提供。"),
            ["search"] = new("search", "搜索", "搜索宿主已就绪，结果由 Core facade 提供。"),
            ["playlists"] = new("playlists", "歌单", "歌单宿主已就绪，持久化状态由 Core facade 提供。"),
            ["favorites"] = new("favorites", "我的收藏", "收藏宿主已就绪，收藏状态由 Core facade 提供。"),
            ["history"] = new("history", "最近播放", "最近播放宿主已就绪，历史记录由 Core facade 提供。"),
            ["downloads"] = new("downloads", "下载管理", "下载宿主已就绪，任务状态由 Core facade 提供。"),
            ["settings"] = new("settings", "设置", "设置宿主已就绪，设备与应用设置将在后续页面接入。"),
        };

    public static ShellRoute Home => Routes["home"];

    public static bool TryGetRoute(string? key, out ShellRoute route)
    {
        if (key is not null && Routes.TryGetValue(key, out var resolved))
        {
            route = resolved;
            return true;
        }

        route = Home;
        return false;
    }

    public static ShellRoute Search(string? query)
    {
        if (string.IsNullOrWhiteSpace(query))
        {
            return Routes["search"];
        }

        return Routes["search"] with
        {
            Description = $"已提交搜索请求：{query.Trim()}。结果由 Core facade 提供。",
        };
    }
}
