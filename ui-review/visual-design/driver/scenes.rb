require "json"
require "open3"

class Scene
  GROUPS = %w[overview facets range chart columns cell preferences corrupt large truncated].freeze

  def self.run_groups(groups, binary, harbor, corrupt, large, output, mode)
    groups.each do |group|
      next if group == "truncated" && mode != "standard"
      dataset = { "corrupt" => corrupt, "large" => large }.fetch(group, harbor)
      Open3.popen2({ "GPUI_A11Y_ACTIVE" => "1" }, binary, dataset) do |input, result, child|
        host = new(input, result, output, mode)
        begin
          host.public_send(group)
        ensure
          input.close
        end
        raise "capture host failed on #{group}" unless child.value.success?
      end
    end
  end

  def initialize(input, result, output, mode)
    @input, @result, @output, @mode = input, result, output, mode
    read_state
    if mode == "high-contrast" || @state["palette"] != "standard"
      command("settings")
      click(mode == "high-contrast" ? "On" : "Off", "RadioButton")
      command("document")
    end
  end

  def read_state
    line = @result.gets or raise "capture host exited"
    @state = JSON.parse(line)
  end

  def command(message)
    @input.puts(message)
    @input.flush
    read_state
  end

  def node(name, role = nil)
    @state.fetch("nodes").find do |n|
      aria = n.fetch("a")
      name === aria["label"] && (!role || aria["role"] == role)
    end
  end

  def require_node(name, role = nil)
    node(name, role) || raise("missing #{role} #{name.inspect}. Available controls: " +
      @state.fetch("nodes").map { |n| n.fetch("a") }
        .reject { |a| %w[GridCell ColumnHeader Row].include?(a["role"]) }
        .map { |a| a["label"] }.compact.join(", "))
  end

  def position(n)
    x0, y0, x1, y1 = n.fetch("rect").map { |v| v / @state.fetch("scale") }
    [(x0 + x1) / 2, (y0 + y1) / 2]
  end

  def click(name, role = nil, button = "left")
    x, y = position(require_node(name, role))
    command("#{button} #{x} #{y}")
  end

  def capture(code, name, role = nil, subset: nil, menu: nil)
    variants = subset || if @mode == "standard"
      %w[light dark].product(%w[active inactive], [1, 2])
    else
      %w[light dark].product(["active"], [2])
    end
    variants.each do |appearance, focus, scale|
      command("output #{appearance} #{focus} #{scale}")
      active = focus == "active"
      if menu && active && !node(name, role)
        menu.call
      end
      # A document menu closes as part of the product's window deactivation.
      if menu && !active
        raise "menu survived window deactivation" if node(name, role)
        target = require_node("Data table", "Grid")
      else
        target = require_node(name, role)
      end
      raise "unexpected installed palette" unless @state["palette"] == @mode
      path = File.join(@output, @mode, "#{appearance}-#{focus}-#{scale}x", code + ".png")
      command("snap #{path}")
      unless @state["appearance"].downcase == appearance && @state["active"] == active && @state["scale"] == scale
        raise "incorrect rendering context for #{path}"
      end
      width, height = File.binread(path, 24)[16, 8].unpack("NN")
      unless [width, height] == [@state["width"] * scale, @state["height"] * scale]
        raise "incorrect physical dimensions for #{path}"
      end
      yield if block_given?
      puts [code, appearance, focus, scale, width, height, target["a"]["label"]].join("\t")
    end
    command("output light active 1")
    menu.call if menu && !node(name, role)
  end

  def overview
    click("Station, row 2", "GridCell")
    capture("01-document", "Inspect column")
    command("key cmd-f")
    command("input survey")
    capture("03-find-substring", "Match 1 of 104")
  end

  def facets
    click("Station, row 1")
    command("action filter")
    capture("10-value-filter", "Deselect all")
    click("Deselect all")
    click("Blackwall, 16 rows")
    click("Apply")
    click("1 filtered")
    command("resize 680 580")
    capture("16b-filter-summary", "Show column Station")
  end

  def range
    click("Water temperature, row 1")
    command("action filter")
    click("Range") unless node("Only missing")
    start = position(require_node(/\[12\.7,.*rows?$/))
    finish = position(require_node(/15\.6\) · 4 rows$/))
    command("drag #{(start + finish).join(" ")}")
    command("resize 680 580")
    capture("14-range-filter", "Only missing")
  end

  def chart
    click("Water temperature, row 1")
    command("action analyze")
    command("action chart")
    click("Chart display options") unless node("Log Y")
    %w[off on].each do |state|
      click("Log Y") if state == "on"
      # Row 81 of the fixture has temperature 16.4 on a 10.2–19.2 domain.
      # Select its actual point so the same screenshots exercise the selection ring.
      x0, y0, x1, y1 = require_node(/Scatter plot, 96 points/)["rect"]
      fraction = if state == "off"
        (16.4 - 10.2) / (19.2 - 10.2)
      else
        Math.log(16.4 / 10.2) / Math.log(19.2 / 10.2)
      end
      x = x0 + 50 + (x1 - x0 - 58) * 80 / 95.0
      y = y0 + 8 + (y1 - y0 - 30) * (1 - fraction)
      command("left #{x} #{y}")
      capture("log-y-#{state}", "Log Y") do
        require_node("Scatter plot, 96 points, row 81 selected")
        checked = require_node("Log Y")["a"]["toggled"]
        raise "unexpected Log Y value #{checked}" unless checked == (state == "on" ? "True" : "False")
      end
    end
  end

  def columns
    header_menu = -> { click("Source", "ColumnHeader", "right") }
    header_menu.call
    capture("18-header-menu", "Sort Ascending", menu: header_menu)
    click("Sort Ascending")
    require_node("1 sorted")
    command("action columns")
    click("Find column")
    command("input Water")
    capture("19b-visible-columns", "Hide matching")
    click("Done")
    command("action columns")
    click("Source", "CheckBox")
    capture("20-hide-confirmation", "Hide", "Button")
    click("Cancel")
    click("Done")
    command("action sort")
    click("Add sort column…")
    click("Find column")
    command("input Water")
    click("Water temperature")
    capture("21-sort", "Sort column")
  end

  def cell
    cell_menu = -> { click("Remarks, row 2", "GridCell", "right") }
    cell_menu.call
    capture("23-cell-menu", "View Cell…", menu: cell_menu)
    click("View Cell…")
    capture("24-cell-preview", "Copy cell value")
  end

  def preferences
    command("settings")
    require_node("High contrast")
    capture("29-settings", "Row height")
  end

  def corrupt
    click("Technical details")
    capture("30b-open-error", "Technical details")
  end

  def large
    table = require_node("Data table")["a"]
    raise "wrong Parquet fixture" unless table["description"] == "2,000,000 rows, 28 columns"
    command("key cmd-f")
    command("input city")
    capture("31b-wide-find", "Match 1 of 2,000,000")
  end

  def truncated
    # "transect" begins past the last visible letter of "Morning transect" at the 76 pt minimum.
    x0, y0, x1, y1 = require_node("Remarks", "ColumnHeader")["rect"]
    y = (y0 + y1) / 2
    command("drag #{x1} #{y} #{x0 + 76} #{y}")
    new_x0, _, new_x1, _ = require_node("Remarks", "ColumnHeader")["rect"]
    raise "column was not narrowed" unless (new_x1 - new_x0).round == 76
    command("key cmd-f")
    command("input transect")
    capture("03b-truncated-ellipsis", "Match 1 of 16", subset: %w[light dark].product(["active"], [1]))
  end
end
