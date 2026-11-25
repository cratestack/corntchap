import type { Meta, StoryObj } from "@storybook/react-vite";
import { ActorAvatar } from "./bits";

const meta = {
  title: "Console/ActorAvatar",
  component: ActorAvatar,
  args: { name: "restaurant", kind: "restaurant" },
  decorators: [(Story) => <div className="flex items-center gap-3 bg-background p-6"><Story /></div>],
} satisfies Meta<typeof ActorAvatar>;
export default meta;
type Story = StoryObj<typeof meta>;

/** The server itself: a squared saffron tile with a chef's hat, never a coloured initial. */
export const Restaurant: Story = {};

export const RestaurantLarge: Story = { args: { className: "size-10 text-[18px]" } };

/** A cook or a person keeps the coloured initial. */
export const Cook: Story = { args: { name: "ada", kind: "agent" } };

/** Beside the others, as in a task's comments. */
export const Together: Story = {
  render: () => (
    <>
      <ActorAvatar name="restaurant" kind="restaurant" />
      <ActorAvatar name="ada" kind="agent" />
      <ActorAvatar name="octo" kind="user" />
    </>
  ),
};
