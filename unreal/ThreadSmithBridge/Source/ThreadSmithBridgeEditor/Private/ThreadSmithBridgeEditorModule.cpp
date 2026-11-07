// Starts the bridge and adds ThreadSmith to the editor:
// - Window > ThreadSmith Wardrobe, and a Wardrobe button on the level toolbar
// - "To ThreadSmith" toolbar button and right-click entries (level + Content Browser)
// - "Send to ThreadSmith" and "Wardrobe" buttons in the Blueprint editor toolbar
//   (like MetaTailor's Send to MT / Receive from MT)

#include "SThreadSmithWardrobe.h"
#include "ThreadSmithBridge.h"

#include "ContentBrowserMenuContexts.h"
#include "Engine/Blueprint.h"
#include "Interfaces/IPluginManager.h"
#include "Styling/SlateStyle.h"
#include "Styling/SlateStyleRegistry.h"
#include "Toolkits/AssetEditorToolkitMenuContext.h"
#include "Engine/SkeletalMesh.h"
#include "Framework/Docking/TabManager.h"
#include "Modules/ModuleManager.h"
#include "Styling/AppStyle.h"
#include "ToolMenus.h"
#include "Widgets/Docking/SDockTab.h"

#define LOCTEXT_NAMESPACE "ThreadSmithBridge"

static const FName WardrobeTabName(TEXT("ThreadSmithWardrobe"));
static const FName StyleName(TEXT("ThreadSmithStyle"));

class FThreadSmithBridgeEditorModule : public IModuleInterface
{
public:
	virtual void StartupModule() override
	{
		FThreadSmithBridge::Get().Start();
		RegisterStyle();

		FGlobalTabmanager::Get()->RegisterNomadTabSpawner(WardrobeTabName, FOnSpawnTab::CreateLambda([](const FSpawnTabArgs&)
		{
			return SNew(SDockTab).TabRole(ETabRole::NomadTab)[SNew(SThreadSmithWardrobe)];
		}))
		.SetDisplayName(LOCTEXT("WardrobeTab", "ThreadSmith Wardrobe"))
		.SetTooltipText(LOCTEXT("WardrobeTabTip", "Outfits from your ThreadSmith library."))
		.SetIcon(FSlateIcon(StyleName, "ThreadSmith.Icon", "ThreadSmith.Icon.Small"))
		.SetMenuType(ETabSpawnerMenuType::Hidden);

		UToolMenus::RegisterStartupCallback(FSimpleMulticastDelegate::FDelegate::CreateRaw(this, &FThreadSmithBridgeEditorModule::RegisterMenus));
	}

	virtual void ShutdownModule() override
	{
		UToolMenus::UnRegisterStartupCallback(this);
		UToolMenus::UnregisterOwner(this);
		if (FSlateApplication::IsInitialized())
		{
			FGlobalTabmanager::Get()->UnregisterNomadTabSpawner(WardrobeTabName);
		}
		FThreadSmithBridge::Get().Stop();
		if (Style.IsValid())
		{
			FSlateStyleRegistry::UnRegisterSlateStyle(*Style);
			Style.Reset();
		}
	}

private:
	TSharedPtr<FSlateStyleSet> Style;

	/** The ThreadSmith logo for toolbar buttons and the Wardrobe tab. */
	void RegisterStyle()
	{
		TSharedPtr<IPlugin> Plugin = IPluginManager::Get().FindPlugin(TEXT("ThreadSmithBridge"));
		if (!Plugin.IsValid())
		{
			return;
		}
		const FString Res = Plugin->GetBaseDir() / TEXT("Resources");
		Style = MakeShared<FSlateStyleSet>(StyleName);
		Style->SetContentRoot(Res);
		Style->Set("ThreadSmith.Icon", new FSlateImageBrush(Res / TEXT("ToolbarIcon40.png"), FVector2D(40.0, 40.0)));
		Style->Set("ThreadSmith.Icon.Small", new FSlateImageBrush(Res / TEXT("ToolbarIcon20.png"), FVector2D(20.0, 20.0)));
		FSlateStyleRegistry::RegisterSlateStyle(*Style);
	}

	/** The Blueprint open in the Blueprint editor this toolbar belongs to. */
	static UBlueprint* EditedBlueprint(const FToolMenuContext& Context)
	{
		if (const UAssetEditorToolkitMenuContext* Ctx = Context.FindContext<UAssetEditorToolkitMenuContext>())
		{
			for (UObject* Obj : Ctx->GetEditingObjects())
			{
				if (UBlueprint* BP = Cast<UBlueprint>(Obj))
				{
					return BP;
				}
			}
		}
		return nullptr;
	}

	static void OpenWardrobe()
	{
		FGlobalTabmanager::Get()->TryInvokeTab(WardrobeTabName);
	}

	void RegisterMenus()
	{
		FToolMenuOwnerScoped OwnerScoped(this);
		const FSlateIcon Icon(StyleName, "ThreadSmith.Icon", "ThreadSmith.Icon.Small");
		const FUIAction OpenAction(FExecuteAction::CreateStatic(&FThreadSmithBridgeEditorModule::OpenWardrobe));
		const FUIAction SendAction(FExecuteAction::CreateLambda([] { FThreadSmithBridge::Get().SendSelectionToThreadSmith(); }));

		// Window menu
		if (UToolMenu* Window = UToolMenus::Get()->ExtendMenu("LevelEditor.MainMenu.Window"))
		{
			FToolMenuSection& Section = Window->FindOrAddSection("ThreadSmith", LOCTEXT("ThreadSmith", "ThreadSmith"));
			Section.AddMenuEntry("ThreadSmithWardrobe", LOCTEXT("Wardrobe", "ThreadSmith Wardrobe"),
				LOCTEXT("WardrobeTip", "Open the ThreadSmith Wardrobe."), Icon, OpenAction);
		}

		// Level editor toolbar
		if (UToolMenu* Toolbar = UToolMenus::Get()->ExtendMenu("LevelEditor.LevelEditorToolBar.User"))
		{
			FToolMenuSection& Section = Toolbar->FindOrAddSection("ThreadSmith");
			Section.AddEntry(FToolMenuEntry::InitToolBarButton("ThreadSmithOpenWardrobe", OpenAction,
				LOCTEXT("WardrobeButton", "Wardrobe"), LOCTEXT("WardrobeTip", "Open the ThreadSmith Wardrobe."), Icon));
			Section.AddEntry(FToolMenuEntry::InitToolBarButton("ThreadSmithSendBody", SendAction,
				LOCTEXT("SendButton", "To ThreadSmith"), LOCTEXT("SendTip", "Send the selected character's body to ThreadSmith."), Icon));
		}

		// Blueprint editor toolbar (like MetaTailor's Send to MT / Receive from MT)
		if (UToolMenu* BPToolbar = UToolMenus::Get()->ExtendMenu("AssetEditor.BlueprintEditor.ToolBar"))
		{
			FToolMenuSection& Section = BPToolbar->FindOrAddSection("ThreadSmith");
			Section.AddEntry(FToolMenuEntry::InitToolBarButton("ThreadSmithSendBlueprint",
				FToolUIActionChoice(FToolMenuExecuteAction::CreateLambda([](const FToolMenuContext& Context)
				{
					if (UBlueprint* BP = EditedBlueprint(Context))
					{
						FThreadSmithBridge::Get().SendBlueprintToThreadSmith(BP);
					}
				})),
				LOCTEXT("SendBP", "Send to ThreadSmith"),
				LOCTEXT("SendBPTip", "Send this character's body and head to ThreadSmith. Outfits you send back are added to this Blueprint."),
				Icon));
			Section.AddEntry(FToolMenuEntry::InitToolBarButton("ThreadSmithBPWardrobe", OpenAction,
				LOCTEXT("BPWardrobe", "ThreadSmith Wardrobe"),
				LOCTEXT("BPWardrobeTip", "Open the Wardrobe to add outfits to this Blueprint. Dock it next to the Blueprint editor."),
				Icon));
		}

		// Right-click on an actor in the level
		if (UToolMenu* ActorMenu = UToolMenus::Get()->ExtendMenu("LevelEditor.ActorContextMenu"))
		{
			FToolMenuSection& Section = ActorMenu->FindOrAddSection("ThreadSmith", LOCTEXT("ThreadSmith", "ThreadSmith"));
			Section.AddMenuEntry("ThreadSmithSendActor", LOCTEXT("SendActor", "Send to ThreadSmith"),
				LOCTEXT("SendActorTip", "Export this character's body and open it in ThreadSmith."), Icon, SendAction);
		}

		// Right-click on a skeletal mesh in the Content Browser
		if (UToolMenu* AssetMenu = UToolMenus::Get()->ExtendMenu("ContentBrowser.AssetContextMenu.SkeletalMesh"))
		{
			FToolMenuSection& Section = AssetMenu->FindOrAddSection("ThreadSmith", LOCTEXT("ThreadSmith", "ThreadSmith"));
			Section.AddMenuEntry("ThreadSmithSendAsset", LOCTEXT("SendAsset", "Send to ThreadSmith"),
				LOCTEXT("SendAssetTip", "Export this skeletal mesh and open it in ThreadSmith as the body."), Icon,
				FToolUIActionChoice(FToolMenuExecuteAction::CreateLambda([](const FToolMenuContext& Context)
				{
					const UContentBrowserAssetContextMenuContext* Ctx = Context.FindContext<UContentBrowserAssetContextMenuContext>();
					if (!Ctx)
					{
						return;
					}
					for (const FAssetData& Asset : Ctx->SelectedAssets)
					{
						if (USkeletalMesh* Mesh = Cast<USkeletalMesh>(Asset.GetAsset()))
						{
							FThreadSmithBridge::Get().SendBodyToThreadSmith(Mesh, nullptr);
							return;
						}
					}
				})));
		}
	}
};

IMPLEMENT_MODULE(FThreadSmithBridgeEditorModule, ThreadSmithBridgeEditor)

#undef LOCTEXT_NAMESPACE
