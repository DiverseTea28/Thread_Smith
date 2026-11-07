// ThreadSmith Bridge editor module: talks to the ThreadSmith app, imports
// outfits, and adds the Wardrobe tab.

using UnrealBuildTool;

public class ThreadSmithBridgeEditor : ModuleRules
{
	public ThreadSmithBridgeEditor(ReadOnlyTargetRules Target) : base(Target)
	{
		PCHUsage = PCHUsageMode.UseExplicitOrSharedPCHs;

		PrivateDependencyModuleNames.AddRange(new string[]
		{
			"Core",
			"CoreUObject",
			"Engine",
			"InputCore",
			"Slate",
			"SlateCore",
			"ApplicationCore",
			"UnrealEd",
			"EditorFramework",
			"ToolMenus",
			"LevelEditor",
			"ContentBrowser",
			"AssetTools",
			"AssetRegistry",
			"DeveloperSettings",
			"Projects",
			"HTTP",
			"HTTPServer",
			"Json",
			"JsonUtilities",
			"ImageWrapper",
			"InterchangeCore",
			"InterchangeEngine",
			"InterchangePipelines",
			"ThreadSmithRuntime",
		});
	}
}
