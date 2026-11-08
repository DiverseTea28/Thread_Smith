#include "SThreadSmithWardrobe.h"

#include "ThreadSmithBridge.h"
#include "ThreadSmithBridgeSettings.h"
#include "ThreadSmithLibrary.h"

#include "Brushes/SlateDynamicImageBrush.h"
#include "HAL/FileManager.h"
#include "HAL/PlatformProcess.h"
#include "Misc/Paths.h"
#include "IImageWrapper.h"
#include "IImageWrapperModule.h"
#include "Misc/FileHelper.h"
#include "Modules/ModuleManager.h"
#include "Styling/AppStyle.h"
#include "Styling/SlateStyleRegistry.h"
#include "Widgets/Images/SImage.h"
#include "Widgets/Input/SButton.h"
#include "Widgets/Input/SComboBox.h"
#include "Widgets/Input/SSearchBox.h"
#include "Widgets/Layout/SBorder.h"
#include "Widgets/Layout/SBox.h"
#include "Widgets/Layout/SScaleBox.h"
#include "Widgets/SBoxPanel.h"
#include "Widgets/Text/STextBlock.h"
#include "Widgets/Views/STableRow.h"

#define LOCTEXT_NAMESPACE "ThreadSmithWardrobe"

namespace
{
	/** Button contents: one of the ThreadSmith icons (Export / Import / Wardrobe), then text. */
	TSharedRef<SWidget> IconLabel(const TCHAR* IconKey, const FText& Label)
	{
		const ISlateStyle* Style = FSlateStyleRegistry::FindSlateStyle(TEXT("ThreadSmithStyle"));
		const FSlateBrush* Brush = Style ? Style->GetOptionalBrush(FName(FString(IconKey) + TEXT(".Button")), nullptr, nullptr) : nullptr;
		return SNew(SHorizontalBox)
			+ SHorizontalBox::Slot().AutoWidth().VAlign(VAlign_Center).Padding(0, 0, Brush ? 6 : 0, 0)
			[
				SNew(SImage).Image(Brush).Visibility(Brush ? EVisibility::Visible : EVisibility::Collapsed)
			]
			+ SHorizontalBox::Slot().AutoWidth().VAlign(VAlign_Center)
			[
				SNew(STextBlock).Text(Label)
			];
	}

	const TCHAR* AllCategories = TEXT("All types");
	// Same list as ThreadSmith (ts-library CATEGORIES).
	const TCHAR* Categories[] = { TEXT("Top"), TEXT("Bottom"), TEXT("Full body"), TEXT("Footwear"), TEXT("Headwear"), TEXT("Gloves"), TEXT("Accessory"), TEXT("Outfit") };
}

void SThreadSmithWardrobe::Construct(const FArguments&)
{
	CategoryOptions.Add(MakeShared<FString>(AllCategories));
	for (const TCHAR* C : Categories)
	{
		CategoryOptions.Add(MakeShared<FString>(C));
	}
	SelectedCategory = CategoryOptions[0];

	LibraryChangedHandle = FThreadSmithBridge::Get().OnLibraryChanged.AddSP(this, &SThreadSmithWardrobe::Refresh);

	ChildSlot
	[
		SNew(SVerticalBox)

		// ---- Top bar: send body, refresh, folder, connection light ----
		+ SVerticalBox::Slot().AutoHeight().Padding(6)
		[
			SNew(SHorizontalBox)
			+ SHorizontalBox::Slot().AutoWidth().Padding(0, 0, 4, 0)
			[
				SNew(SButton)
				.ToolTipText(LOCTEXT("SendBodyTip", "Exports the selected character's body and opens it in ThreadSmith to make clothes for it."))
				.OnClicked(this, &SThreadSmithWardrobe::OnSendBody)
				[
					IconLabel(TEXT("ThreadSmith.Export"), LOCTEXT("SendBody", "Send selected character to ThreadSmith"))
				]
			]
			+ SHorizontalBox::Slot().AutoWidth().Padding(0, 0, 4, 0)
			[
				SNew(SButton).Text(LOCTEXT("Refresh", "Refresh")).OnClicked(this, &SThreadSmithWardrobe::OnRefresh)
			]
			+ SHorizontalBox::Slot().AutoWidth()
			[
				SNew(SButton).Text(LOCTEXT("Folder", "Open library folder")).OnClicked(this, &SThreadSmithWardrobe::OnOpenFolder)
			]
			+ SHorizontalBox::Slot().FillWidth(1.0f).HAlign(HAlign_Right).VAlign(VAlign_Center)
			[
				SNew(STextBlock)
				.Text_Lambda([]
				{
					return FThreadSmithBridge::Get().IsAppConnected()
						? LOCTEXT("Connected", "ThreadSmith connected")
						: LOCTEXT("NotConnected", "ThreadSmith not running");
				})
				.ColorAndOpacity_Lambda([]
				{
					return FThreadSmithBridge::Get().IsAppConnected() ? FSlateColor(FLinearColor(0.3f, 0.85f, 0.4f)) : FSlateColor::UseSubduedForeground();
				})
			]
		]

		// ---- Search + type filter ----
		+ SVerticalBox::Slot().AutoHeight().Padding(6, 0, 6, 6)
		[
			SNew(SHorizontalBox)
			+ SHorizontalBox::Slot().FillWidth(1.0f).Padding(0, 0, 4, 0)
			[
				SNew(SSearchBox)
				.HintText(LOCTEXT("Search", "Search outfits"))
				.OnTextChanged_Lambda([this](const FText& T) { SearchText = T.ToString(); ApplyFilter(); })
			]
			+ SHorizontalBox::Slot().AutoWidth()
			[
				SNew(SComboBox<TSharedPtr<FString>>)
				.OptionsSource(&CategoryOptions)
				.OnGenerateWidget_Lambda([](TSharedPtr<FString> V) { return SNew(STextBlock).Text(FText::FromString(*V)); })
				.OnSelectionChanged_Lambda([this](TSharedPtr<FString> V, ESelectInfo::Type)
				{
					if (V.IsValid())
					{
						SelectedCategory = V;
						ApplyFilter();
					}
				})
				.InitiallySelectedItem(SelectedCategory)
				[
					SNew(STextBlock).Text_Lambda([this] { return FText::FromString(*SelectedCategory); })
				]
			]
		]

		// ---- The grid ----
		+ SVerticalBox::Slot().FillHeight(1.0f).Padding(6, 0)
		[
			SNew(SBorder)
			.BorderImage(FAppStyle::GetBrush("ToolPanel.GroupBorder"))
			[
				SAssignNew(TileView, STileView<FItem>)
				.ListItemsSource(&ShownOutfits)
				.OnGenerateTile(this, &SThreadSmithWardrobe::MakeTile)
				.OnMouseButtonDoubleClick(this, &SThreadSmithWardrobe::OnTileDoubleClick)
				.SelectionMode(ESelectionMode::Single)
				.ItemWidth(150.0f)
				.ItemHeight(186.0f)
			]
		]

		// ---- Actions on the chosen outfit ----
		+ SVerticalBox::Slot().AutoHeight().Padding(6)
		[
			SNew(SHorizontalBox)
			+ SHorizontalBox::Slot().AutoWidth().Padding(0, 0, 4, 0)
			[
				SNew(SButton)
				.Content()
				[
					IconLabel(TEXT("ThreadSmith.Wardrobe"), LOCTEXT("AddBP", "Add to character"))
				]
				.ToolTipText(LOCTEXT("AddBPTip", "Adds the outfit to the character's Blueprint as a component (like MetaTailor), so every copy of that character wears it. Uses the Blueprint you have open, or the selected character's Blueprint. To take it off, delete the component in the Blueprint. Double-clicking a tile does the same."))
				.IsEnabled_Lambda([this] { return GetSelected().IsValid(); })
				.OnClicked(this, &SThreadSmithWardrobe::OnAddToBlueprint)
			]
			+ SHorizontalBox::Slot().AutoWidth()
			[
				SNew(SButton)
				.Content()
				[
					IconLabel(TEXT("ThreadSmith.Import"), LOCTEXT("Import", "Import only"))
				]
				.ToolTipText(LOCTEXT("ImportTip", "Imports (or re-imports) the outfit into the project without putting it on anyone."))
				.IsEnabled_Lambda([this] { return GetSelected().IsValid(); })
				.OnClicked(this, &SThreadSmithWardrobe::OnImport)
			]
			+ SHorizontalBox::Slot().FillWidth(1.0f).HAlign(HAlign_Right).VAlign(VAlign_Center)
			[
				SNew(STextBlock)
				.ColorAndOpacity(FSlateColor::UseSubduedForeground())
				.Text_Lambda([this]
				{
					return FText::Format(LOCTEXT("Count", "{0} of {1} outfits"), ShownOutfits.Num(), AllOutfits.Num());
				})
			]
		]
	];

	Refresh();
}

SThreadSmithWardrobe::~SThreadSmithWardrobe()
{
	FThreadSmithBridge::Get().OnLibraryChanged.Remove(LibraryChangedHandle);
}

void SThreadSmithWardrobe::Refresh()
{
	AllOutfits = ThreadSmithLibrary::List(GetDefault<UThreadSmithBridgeSettings>()->GetLibraryFolder());
	ApplyFilter();
}

void SThreadSmithWardrobe::ApplyFilter()
{
	ShownOutfits.Reset();
	const bool bAllTypes = !SelectedCategory.IsValid() || *SelectedCategory == AllCategories;
	for (const FItem& O : AllOutfits)
	{
		if (!bAllTypes && O->Category != *SelectedCategory)
		{
			continue;
		}
		if (!SearchText.IsEmpty() && !O->Name.Contains(SearchText) && !O->Character.Contains(SearchText))
		{
			continue;
		}
		ShownOutfits.Add(O);
	}
	if (TileView.IsValid())
	{
		TileView->RequestListRefresh();
	}
}

SThreadSmithWardrobe::FItem SThreadSmithWardrobe::GetSelected() const
{
	if (!TileView.IsValid())
	{
		return nullptr;
	}
	TArray<FItem> Sel = TileView->GetSelectedItems();
	return Sel.Num() > 0 ? Sel[0] : nullptr;
}

const FSlateBrush* SThreadSmithWardrobe::GetThumbnail(const FThreadSmithOutfit& Outfit)
{
	const FString Path = Outfit.ThumbnailPath();
	if (TSharedPtr<FSlateDynamicImageBrush>* Found = Thumbnails.Find(Path))
	{
		return Found->IsValid() ? Found->Get() : FAppStyle::GetBrush("ClassThumbnail.SkeletalMesh");
	}

	TSharedPtr<FSlateDynamicImageBrush> Brush;
	TArray<uint8> FileData;
	if (FFileHelper::LoadFileToArray(FileData, *Path))
	{
		IImageWrapperModule& Wrappers = FModuleManager::LoadModuleChecked<IImageWrapperModule>("ImageWrapper");
		TSharedPtr<IImageWrapper> Png = Wrappers.CreateImageWrapper(EImageFormat::PNG);
		TArray<uint8> Bgra;
		if (Png.IsValid() && Png->SetCompressed(FileData.GetData(), FileData.Num()) && Png->GetRaw(ERGBFormat::BGRA, 8, Bgra))
		{
			// Unique name per file version, so a re-saved outfit shows its new picture.
			const FName Name(*FString::Printf(TEXT("ThreadSmith_%s_%lld"), *Outfit.Id, Outfit.Modified));
			Brush = FSlateDynamicImageBrush::CreateWithImageData(Name, FVector2D(Png->GetWidth(), Png->GetHeight()), Bgra);
		}
	}
	Thumbnails.Add(Path, Brush);
	return Brush.IsValid() ? Brush.Get() : FAppStyle::GetBrush("ClassThumbnail.SkeletalMesh");
}

TSharedRef<ITableRow> SThreadSmithWardrobe::MakeTile(FItem Item, const TSharedRef<STableViewBase>& Owner)
{
	return SNew(STableRow<FItem>, Owner)
		.Padding(4.0f)
		.ToolTipText(FText::Format(LOCTEXT("TileTip", "{0}\n{1} · made for {2}\n{3} triangles\n{4}"),
			FText::FromString(Item->Name), FText::FromString(Item->Category), FText::FromString(Item->Character),
			Item->Triangles, FText::FromString(Item->Dir)))
		[
			SNew(SVerticalBox)
			+ SVerticalBox::Slot().AutoHeight()
			[
				SNew(SBox).WidthOverride(140.0f).HeightOverride(140.0f)
				[
					SNew(SScaleBox).Stretch(EStretch::ScaleToFit)
					[
						SNew(SImage).Image(GetThumbnail(*Item))
					]
				]
			]
			+ SVerticalBox::Slot().AutoHeight().Padding(2, 4, 2, 0)
			[
				SNew(STextBlock).Text(FText::FromString(Item->Name)).OverflowPolicy(ETextOverflowPolicy::Ellipsis)
			]
			+ SVerticalBox::Slot().AutoHeight().Padding(2, 0)
			[
				SNew(STextBlock)
				.Text(FText::FromString(Item->Category))
				.ColorAndOpacity(FSlateColor::UseSubduedForeground())
				.Font(FAppStyle::GetFontStyle("SmallFont"))
			]
		];
}

void SThreadSmithWardrobe::OnTileDoubleClick(FItem Item)
{
	if (Item.IsValid())
	{
		FThreadSmithBridge::Get().AddOutfitToTargetBlueprint(*Item);
	}
}

FReply SThreadSmithWardrobe::OnAddToBlueprint()
{
	if (FItem Item = GetSelected())
	{
		FThreadSmithBridge::Get().AddOutfitToTargetBlueprint(*Item);
	}
	return FReply::Handled();
}

FReply SThreadSmithWardrobe::OnImport()
{
	if (FItem Item = GetSelected())
	{
		FString Error;
		FThreadSmithBridge::Get().ImportOutfit(*Item, /*bForceReimport*/ true, Error);
	}
	return FReply::Handled();
}

FReply SThreadSmithWardrobe::OnSendBody()
{
	FThreadSmithBridge::Get().SendSelectionToThreadSmith();
	return FReply::Handled();
}

FReply SThreadSmithWardrobe::OnOpenFolder()
{
	const FString Folder = GetDefault<UThreadSmithBridgeSettings>()->GetLibraryFolder();
	IFileManager::Get().MakeDirectory(*FPaths::Combine(Folder, TEXT("Outfits")), true);
	FPlatformProcess::ExploreFolder(*Folder);
	return FReply::Handled();
}

FReply SThreadSmithWardrobe::OnRefresh()
{
	Thumbnails.Reset();
	Refresh();
	return FReply::Handled();
}

#undef LOCTEXT_NAMESPACE
