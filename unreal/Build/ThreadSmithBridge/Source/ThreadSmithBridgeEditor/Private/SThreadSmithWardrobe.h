// The ThreadSmith Wardrobe tab: a thumbnail grid of the outfit library.

#pragma once

#include "CoreMinimal.h"
#include "Widgets/SCompoundWidget.h"
#include "Widgets/Views/STileView.h"

struct FThreadSmithOutfit;
struct FSlateDynamicImageBrush;

class SThreadSmithWardrobe : public SCompoundWidget
{
public:
	SLATE_BEGIN_ARGS(SThreadSmithWardrobe) {}
	SLATE_END_ARGS()

	void Construct(const FArguments& InArgs);
	virtual ~SThreadSmithWardrobe() override;

private:
	using FItem = TSharedPtr<FThreadSmithOutfit>;

	void Refresh();
	void ApplyFilter();
	FItem GetSelected() const;
	const struct FSlateBrush* GetThumbnail(const FThreadSmithOutfit& Outfit);

	TSharedRef<class ITableRow> MakeTile(FItem Item, const TSharedRef<class STableViewBase>& Owner);
	void OnTileDoubleClick(FItem Item);

	FReply OnAddToBlueprint();
	FReply OnImport();
	FReply OnSendBody();
	FReply OnOpenFolder();
	FReply OnRefresh();

	TArray<FItem> AllOutfits;
	TArray<FItem> ShownOutfits;
	TSharedPtr<STileView<FItem>> TileView;

	FString SearchText;
	TArray<TSharedPtr<FString>> CategoryOptions;
	TSharedPtr<FString> SelectedCategory;

	/** Thumbnail brushes by file path. */
	TMap<FString, TSharedPtr<FSlateDynamicImageBrush>> Thumbnails;

	FDelegateHandle LibraryChangedHandle;
};
